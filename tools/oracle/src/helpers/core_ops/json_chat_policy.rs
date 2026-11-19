use std::collections::BTreeMap;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::{
    JSONProviderRegistry, ProviderConfig, ProviderDescriptor,
};
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::json_provider_policy::{ChatCapabilities, ChatPolicy};
use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod tests;

// OpenAIGPTConfig.get_supported_openai_params, without the catalog check no case exercises
const BASE_PARAMS: [&str; 30] = [
    "frequency_penalty",
    "logit_bias",
    "logprobs",
    "top_logprobs",
    "max_tokens",
    "max_completion_tokens",
    "modalities",
    "prediction",
    "n",
    "presence_penalty",
    "seed",
    "stop",
    "stream",
    "stream_options",
    "temperature",
    "top_p",
    "tools",
    "tool_choice",
    "function_call",
    "functions",
    "max_retries",
    "extra_headers",
    "parallel_tool_calls",
    "audio",
    "web_search_options",
    "service_tier",
    "safety_identifier",
    "prompt_cache_key",
    "prompt_cache_retention",
    "store",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Identity,
    Supported,
    Params,
    Url,
    Messages,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaseInput {
    calls: Vec<Call>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    kind: Kind,
    model: String,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    supports_function_calling: bool,
    #[serde(default)]
    supports_reasoning: bool,
    #[serde(default)]
    descriptor: Option<ProviderConfig>,
    #[serde(default)]
    non_default: Object,
    #[serde(default)]
    optional: Object,
    #[serde(default)]
    messages: Vec<ChatMessage>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = CaseInput;

    fn run(input: &Self::Input) -> HelperResult {
        let registry = JSONProviderRegistry::load();
        let mut results: Vec<Value> = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            results.push(run_call(call, &registry)?);
        }
        Ok(Value::Array(results))
    }
}

fn run_call(call: &Call, registry: &JSONProviderRegistry) -> Result<Value, Box<HelperError>> {
    let (slug, model) = split_model(&call.model)?;
    let policy = policy(call, registry, &slug)?;
    match call.kind {
        Kind::Identity => identity(call, &policy, &slug, &model),
        Kind::Supported => {
            let supported = policy.supported_params(&parent_params(&model), capabilities(call));
            serde_json::to_value(supported)
                .map_err(|error| helper_error("ValueError", error.to_string()))
        }
        Kind::Params => {
            let supported = policy.supported_params(&parent_params(&model), capabilities(call));
            let mut optional = call.optional.clone();
            policy
                .map_params(&call.non_default, &mut optional, &supported)
                .map_err(|error| helper_error("TypeError", error.to_string()))?;
            serde_json::to_value(optional)
                .map_err(|error| helper_error("ValueError", error.to_string()))
        }
        Kind::Url => Ok(Value::String(policy.complete_url(call.api_base.as_deref()))),
        Kind::Messages => messages(call, &policy),
    }
}

// the model prefix names the provider, and LiteLLM hands every later step the stripped model
fn split_model(model: &str) -> Result<(String, String), Box<HelperError>> {
    let Some((slug, rest)) = model.split_once('/') else {
        return Err(helper_error(
            "ValueError",
            format!("the case model {model} carries no provider prefix"),
        ));
    };
    Ok((slug.to_owned(), rest.to_owned()))
}

fn provider_id(slug: &str) -> Result<ProviderId, Box<HelperError>> {
    slug.parse()
        .map_err(|error| helper_error("ValueError", format!("{slug}: {error}")))
}

fn policy(
    call: &Call,
    registry: &JSONProviderRegistry,
    slug: &str,
) -> Result<ChatPolicy, Box<HelperError>> {
    let provider = provider_id(slug)?;
    if let Some(config) = &call.descriptor {
        let descriptor = ProviderDescriptor {
            provider,
            config: config.clone(),
        };
        return Ok(ChatPolicy { descriptor });
    }
    registry
        .providers
        .get(&provider)
        .cloned()
        .map(|descriptor| ChatPolicy { descriptor })
        .ok_or_else(|| helper_error("ValueError", format!("{slug} is not a JSON provider")))
}

const fn capabilities(call: &Call) -> ChatCapabilities {
    ChatCapabilities {
        supports_function_calling: call.supports_function_calling,
        supports_reasoning: call.supports_reasoning,
    }
}

fn parent_params(model: &str) -> Vec<&'static str> {
    let mut params = BASE_PARAMS.to_vec();
    if model != "gpt-3.5-turbo-16k" && model != "gpt-4" {
        params.push("response_format");
    }
    params
}

fn identity(
    call: &Call,
    policy: &ChatPolicy,
    slug: &str,
    model: &str,
) -> Result<Value, Box<HelperError>> {
    let explicit = call.api_key.clone().map(SecretString::from);
    let (base, key) = policy.resolve_provider_info(
        call.api_base.as_deref(),
        explicit.as_ref(),
        &call.environment,
    );
    Ok(json!({
        "model": model,
        "provider": slug,
        "base": base,
        "key": key.map(|secret| secret.expose_secret().to_owned()),
    }))
}

fn messages(call: &Call, policy: &ChatPolicy) -> Result<Value, Box<HelperError>> {
    let transformed = policy
        .transform_messages(&call.messages)
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
    serde_json::to_value(transformed).map_err(|error| helper_error("ValueError", error.to_string()))
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
