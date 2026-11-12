use betterlite_config::sections::mistral_options::Settings as EnvironmentSettings;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

mod tests;

pub const DEFAULT_API_BASE: &str = "https://api.mistral.ai/v1";
// the fixture ends at the final newline, so the 8-space closing indent of LiteLLM's literal is appended here
pub const REASONING_SYSTEM_PROMPT: &str = concat!(
    include_str!("../fixtures/mistral_reasoning.txt"),
    "        "
);
pub const STANDALONE_MAX_RECURSE_DEPTH: i64 = 10;

const BASE_OPENAI_PARAMS: [&str; 11] = [
    "stream",
    "temperature",
    "top_p",
    "max_tokens",
    "max_completion_tokens",
    "tools",
    "tool_choice",
    "seed",
    "stop",
    "response_format",
    "parallel_tool_calls",
];

// LiteLLM's class attributes all start as None, so a fresh config exports nothing and a call sends only its own params
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MistralSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub random_seed: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safe_prompt: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Value>,
}

// B323 decides which declared effort is nearest, so B036 injects it and this leaf crate never imports the router
pub trait ReasoningEffortSelector {
    // None means the catalog declares no levels for the model, so the requested effort passes through
    fn accepted_effort(&self, model: &str, requested: &str) -> Option<String>;
}

#[must_use]
pub fn supported_openai_params(model: &str, supports_reasoning: bool) -> Vec<&'static str> {
    let magistral = model.to_lowercase().contains("magistral");
    let mut params = BASE_OPENAI_PARAMS.to_vec();
    if magistral {
        params.push("thinking");
    }
    if magistral || supports_reasoning {
        params.push("reasoning_effort");
    }
    params
}

// The pairs arrive in encounter order, so the last spelling of a max-token name wins as it does in Python
pub fn map_openai_params(
    non_default_params: &[(String, Value)],
    optional_params: &mut Map<String, Value>,
    model: &str,
    selector: &dyn ReasoningEffortSelector,
    max_depth: i64,
) {
    let magistral = model.to_lowercase().contains("magistral");
    for (name, value) in non_default_params {
        match name.as_str() {
            "max_tokens" | "max_completion_tokens" => {
                optional_params.insert(String::from("max_tokens"), value.clone());
            }
            "tools" => {
                optional_params.insert(
                    String::from("tools"),
                    clean_tool_schema_for_mistral(value, max_depth),
                );
            }
            // Python's `value is True` accepts the bool itself only, so 1 and "true" do not pass
            "stream" if *value == Value::Bool(true) => {
                optional_params.insert(String::from("stream"), Value::Bool(true));
            }
            "temperature" | "top_p" | "stop" | "response_format" | "parallel_tool_calls" => {
                optional_params.insert(name.clone(), value.clone());
            }
            "tool_choice" => {
                if let Some(mapped) = tool_choice_value(value) {
                    optional_params.insert(String::from("tool_choice"), mapped);
                }
            }
            "seed" => {
                let mut extra_body = Map::new();
                extra_body.insert(String::from("random_seed"), value.clone());
                optional_params.insert(String::from("extra_body"), Value::Object(extra_body));
            }
            "reasoning_effort" | "thinking" if magistral => {
                optional_params.insert(String::from("_add_reasoning_prompt"), Value::Bool(true));
            }
            "reasoning_effort" => {
                let accepted = resolve_reasoning_effort(selector, model, value);
                optional_params.insert(String::from("reasoning_effort"), accepted);
            }
            _ => {}
        }
    }
}

// Python's tool-choice mapper leaves every non-string choice untouched
fn tool_choice_value(choice: &Value) -> Option<Value> {
    let Value::String(text) = choice else {
        return None;
    };
    let mapped = match text.as_str() {
        "auto" | "none" => text.clone(),
        _ => String::from("any"),
    };
    Some(Value::String(mapped))
}

fn resolve_reasoning_effort(
    selector: &dyn ReasoningEffortSelector,
    model: &str,
    value: &Value,
) -> Value {
    let Some(requested) = value.as_str() else {
        return value.clone();
    };
    let Some(accepted) = selector.accepted_effort(model, requested) else {
        return value.clone();
    };
    if accepted != requested {
        tracing::debug!(
            provider = "mistral",
            model,
            requested,
            %accepted,
            "the declared reasoning efforts round the requested effort"
        );
    }
    Value::String(accepted)
}

#[must_use]
pub fn clean_tool_schema_for_mistral(tools: &Value, max_depth: i64) -> Value {
    let mut cleaned = tools.clone();
    remove_json_schema_refs(&mut cleaned, max_depth);
    cleaned
}

// LiteLLM's utility defaults to a depth of 10, while Mistral passes its env-resolved DEFAULT_MAX_RECURSE_DEPTH
pub fn remove_json_schema_refs(schema: &mut Value, max_depth: i64) {
    if max_depth <= 0 {
        return;
    }
    match schema {
        Value::Object(fields) => {
            fields.remove("$id");
            fields.remove("$schema");
            for value in fields.values_mut() {
                remove_json_schema_refs(value, max_depth.saturating_sub(1));
            }
        }
        Value::Array(items) => {
            for item in items {
                remove_json_schema_refs(item, max_depth.saturating_sub(1));
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

#[must_use]
pub fn provider_info(
    api_base: Option<&str>,
    api_key: Option<&str>,
    environment: &EnvironmentSettings,
) -> (String, Option<SecretString>) {
    let mut base = api_base
        .map(str::to_owned)
        .or_else(|| environment.azure_api_base.clone())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    // the suffix test is literal, so a base ending in /v1/ still gains another /v1
    if !base.ends_with("/v1") {
        base.push_str("/v1");
    }
    let key = api_key
        .map(SecretString::from)
        .or_else(|| environment.azure_api_key.clone())
        .or_else(|| environment.api_key.clone());
    (base, key)
}
