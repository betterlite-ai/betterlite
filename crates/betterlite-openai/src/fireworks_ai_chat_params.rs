use std::future::Future;

use betterlite_context::error::ContextError;
use betterlite_error::error::details::ResponseDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use serde_json::Value;
use thiserror::Error;

use crate::openai_base_config::Object;

pub mod extra_body;
pub mod mapping;
pub mod settings;
pub mod supported;

mod tests;

pub const PROVIDER: &str = "fireworks_ai";

pub const EFFORT_KWARG_KEYS: [&str; 4] = [
    "enable_thinking",
    "thinking",
    "reasoning_budget",
    "low_effort",
];

pub const NIM_VLLM_STRIP_PARAMS: [&str; 16] = [
    "add_generation_prompt",
    "add_special_tokens",
    "allowed_token_ids",
    "bad_words",
    "best_of",
    "continue_final_message",
    "detokenize",
    "guided_decoding_backend",
    "guided_regex",
    "include_reasoning",
    "include_stop_str_in_output",
    "nvext",
    "skip_special_tokens",
    "spaces_between_special_tokens",
    "stop_token_ids",
    "use_beam_search",
];

const EXTRA_BODY_CONSUMED_PARAMS: [&str; 21] = [
    "add_generation_prompt",
    "add_special_tokens",
    "allowed_token_ids",
    "bad_words",
    "best_of",
    "chat_template_kwargs",
    "continue_final_message",
    "detokenize",
    "guided_choice",
    "guided_decoding_backend",
    "guided_grammar",
    "guided_json",
    "guided_regex",
    "include_reasoning",
    "include_stop_str_in_output",
    "nvext",
    "skip_special_tokens",
    "spaces_between_special_tokens",
    "stop_token_ids",
    "truncate_prompt_tokens",
    "use_beam_search",
];

// Each method is LiteLLM's litellm.utils.supports_* wrapper for provider fireworks_ai; B2221 owns the lookup.
pub trait FireworksCapabilities {
    fn supports_function_calling(&mut self, model: &str) -> impl Future<Output = bool> + Send;

    fn supports_tool_choice(&mut self, model: &str) -> impl Future<Output = bool> + Send;

    fn supports_reasoning(&mut self, model: &str) -> impl Future<Output = bool> + Send;
}

#[derive(Debug, Error)]
pub enum FireworksChatParamsError {
    #[error(
        "Fireworks AI chat completions does not support specifying both `thinking` and `reasoning_effort` in the same request"
    )]
    ThinkingConflict { model: String },
    #[error("response_format must be an object when tools are set")]
    ResponseFormatNotObject,
    #[error("the existing tools value is not a list")]
    ToolsNotList,
    #[error("response_format cannot be translated into a tool")]
    ResponseFormat {
        #[source]
        source: ContextError,
    },
    #[error("a translated parameter cannot be encoded as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Fireworks AI chat settings could not be read")]
    Settings {
        #[source]
        source: serde_json::Error,
    },
}

impl FireworksChatParamsError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::ThinkingConflict { .. }
            | Self::ResponseFormatNotObject
            | Self::ToolsNotList
            | Self::ResponseFormat { .. } => Some(400),
            Self::Encode { .. } | Self::Settings { .. } => None,
        }
    }

    #[must_use]
    pub fn class(&self) -> Option<ErrorClass> {
        let model = match self {
            Self::ThinkingConflict { model } => Some(model.clone()),
            Self::ResponseFormatNotObject
            | Self::ToolsNotList
            | Self::ResponseFormat { .. }
            | Self::Encode { .. }
            | Self::Settings { .. } => None,
        };
        let context = ErrorContext {
            message: self.to_string(),
            model,
            provider: Some(PROVIDER.to_owned()),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        };
        match self {
            Self::ThinkingConflict { .. }
            | Self::ResponseFormatNotObject
            | Self::ToolsNotList
            | Self::ResponseFormat { .. } => Some(ErrorClass::BadRequest(Box::new(
                ResponseDetails::from(context),
            ))),
            Self::Encode { .. } | Self::Settings { .. } => None,
        }
    }
}

// LiteLLM's effort_from_chat_template_kwargs: the first bool of enable_thinking/thinking, then a positive budget, then low_effort.
#[must_use]
pub fn effort_from_template_kwargs(kwargs: &Object) -> Option<Value> {
    if first_bool(kwargs, &["enable_thinking", "thinking"]) == Some(false) {
        return Some(Value::String(String::from("none")));
    }
    if let Some(tokens) = kwargs
        .get("reasoning_budget")
        .and_then(whole_positive_tokens)
    {
        return Some(tokens);
    }
    if first_bool(kwargs, &["low_effort"]) == Some(true) {
        return Some(Value::String(String::from("low")));
    }
    None
}

fn first_bool(kwargs: &Object, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|key| match kwargs.get(*key) {
        Some(Value::Bool(value)) => Some(*value),
        _ => None,
    })
}

fn whole_positive_tokens(value: &Value) -> Option<Value> {
    let number = value.as_number()?;
    if let Some(tokens) = number.as_i64() {
        return (tokens > 0).then_some(Value::from(tokens));
    }
    // a negative integer is caught by the i64 arm, so a u64 here is always positive
    if let Some(tokens) = number.as_u64() {
        return Some(Value::from(tokens));
    }
    let parsed = number.as_f64()?;
    if !parsed.is_finite() || parsed <= 0.0_f64 {
        return None;
    }
    let truncated = parsed.trunc();
    serde_json::from_str::<Value>(&format!("{truncated}")).ok()
}

// LiteLLM's translate_guided_params: the first non-null guided key becomes the response_format the call sends.
#[must_use]
pub fn translate_guided_params(extra_body: &Object, optional_params: &Object) -> Option<Value> {
    let guided_keys = ["guided_json", "guided_grammar", "guided_choice"];
    let has_guided = guided_keys
        .iter()
        .any(|key| extra_body.get(*key).is_some_and(|value| !value.is_null()));
    if !has_guided {
        return None;
    }
    if optional_params.contains_key("response_format") || extra_body.contains_key("response_format")
    {
        tracing::debug!(
            "fireworks_ai ignoring guided decoding params; explicit response_format takes precedence"
        );
        return None;
    }
    if let Some(schema) = extra_body
        .get("guided_json")
        .filter(|value| !value.is_null())
    {
        return Some(json_schema_response_format(schema, "response"));
    }
    if let Some(grammar) = extra_body
        .get("guided_grammar")
        .filter(|value| !value.is_null())
    {
        return Some(grammar_response_format(grammar));
    }
    let choice = extra_body
        .get("guided_choice")
        .filter(|value| !value.is_null())?;
    Some(choice_response_format(choice))
}

fn grammar_response_format(grammar: &Value) -> Value {
    let mut format = Object::new();
    format.insert(String::from("type"), Value::String(String::from("grammar")));
    format.insert(String::from("grammar"), grammar.clone());
    Value::Object(format)
}

fn choice_response_format(choice: &Value) -> Value {
    let mut schema = Object::new();
    schema.insert(String::from("type"), Value::String(String::from("string")));
    schema.insert(String::from("enum"), choice.clone());
    json_schema_response_format(&Value::Object(schema), "choice")
}

fn json_schema_response_format(schema: &Value, name: &str) -> Value {
    let mut json_schema = Object::new();
    json_schema.insert(String::from("name"), Value::String(name.to_owned()));
    json_schema.insert(String::from("schema"), schema.clone());
    let mut format = Object::new();
    format.insert(
        String::from("type"),
        Value::String(String::from("json_schema")),
    );
    format.insert(String::from("json_schema"), Value::Object(json_schema));
    Value::Object(format)
}
