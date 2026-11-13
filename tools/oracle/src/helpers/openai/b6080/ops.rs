use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::decisions::ir::{DecisionResponse, DecisionUsage};
use betterlite_models::decisions::{Extra, Json as ModelJson, TokenCount};
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig;
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};
use betterlite_openai::openrouter_chat::error::{OpenRouterError, get_error_class};
use betterlite_openai::openrouter_chat::policy::{BaseRequest, ChatPolicy};
use betterlite_openai::openrouter_chat::OpenrouterConfig;
use betterlite_openai::openrouter_decisions::OpenRouterDecisionsConfig;
use betterlite_openai::openrouter_embedding::SUPPORTED_OPENAI_PARAMS;
use betterlite_openai::openrouter_responses::{
    PROVIDER, SUPPORTS_NATIVE_WEBSOCKET, get_complete_url as responses_url,
    validate_environment as responses_headers,
};
use serde_json::{Map, Number, Value, json};

use crate::helpers::{HelperResult, OrHelperError as _};

use super::input::{DecisionsCase, ErrorCase, Input, ResponsesCase};

struct OpenAiPolicy;

impl ChatPolicy for OpenAiPolicy {
    fn supported_params(&self, model: &str) -> Vec<&'static str> {
        openai_config().supported_params(model, &empty_catalog())
    }

    fn map_params(&self, _model: &str, _requested: &Object, _optional_params: &mut Object) {}

    fn remove_cache_control(&self, _messages: &mut Vec<ChatMessage>, _optional_params: &mut Object) {}

    fn request_body(&self, _request: BaseRequest<'_>) -> Result<Object, OpenRouterError> {
        Err(unavailable())
    }

    fn response_body(&self, _raw: &[u8]) -> Result<Value, OpenRouterError> {
        Err(unavailable())
    }
}

fn unavailable() -> OpenRouterError {
    OpenRouterError {
        message: String::from("this helper operation does not build a request"),
        status_code: 500,
        headers: Headers::new(),
    }
}

// B668's GPT base list; ConfigFamily::Other keeps the OpenAIConfig-only unknown-model reasoning_effort out
fn openai_config() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

fn empty_catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn config() -> OpenrouterConfig<OpenAiPolicy> {
    OpenrouterConfig {
        policy: OpenAiPolicy,
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Supported { models } => Ok(json!(
            models
                .iter()
                .map(|model| config().supported_params(model))
                .collect::<Vec<_>>()
        )),
        Input::Embedding => Ok(json!({"supported": SUPPORTED_OPENAI_PARAMS})),
        Input::Responses { cases } => responses(cases),
        Input::Decisions { cases } => Ok(decisions(cases)),
        Input::ErrorClass { cases } => Ok(Value::Array(cases.iter().map(error_class).collect())),
    }
}

fn responses(cases: &[ResponsesCase]) -> HelperResult {
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let environment: BTreeMap<String, String> = case.environment.clone();
        let headers = responses_headers(
            case.headers.clone(),
            case.api_key.as_deref(),
            case.global_api_key.as_deref(),
            &environment,
        )
        .or_value_error()?;
        results.push(json!({
            "headers": headers,
            "url": responses_url(
                case.api_base.as_deref(),
                case.global_api_base.as_deref(),
                &environment,
            ),
            "provider": PROVIDER.to_string(),
            "websocket": SUPPORTS_NATIVE_WEBSOCKET,
        }));
    }
    Ok(Value::Array(results))
}

fn decisions(cases: &[DecisionsCase]) -> Value {
    let decisions = OpenRouterDecisionsConfig;
    json!({
        "path": OpenRouterDecisionsConfig::PATH,
        "api_key_env": OpenRouterDecisionsConfig::API_KEY_ENV,
        "api_base_env": OpenRouterDecisionsConfig::API_BASE_ENV,
        "default_api_base": decisions.get_default_api_base(),
        "complete_url": decisions.get_complete_url("https://openrouter.ai/api/v1", "m"),
        "costs": cases.iter().map(|case| cost(&decisions, &case.cost)).collect::<Vec<_>>(),
    })
}

fn cost(decisions: &OpenRouterDecisionsConfig, cost: &Value) -> Value {
    let mut extra = Extra::new();
    extra.insert(String::from("cost"), model_json(cost));
    let response = DecisionResponse {
        model: None,
        answers: Vec::new(),
        usage: DecisionUsage {
            input_tokens: TokenCount::default(),
            output_tokens: TokenCount::default(),
            cached_tokens: TokenCount::default(),
            cache_write_tokens: TokenCount::default(),
            reasoning_tokens: TokenCount::default(),
            extra,
        },
        extra: Extra::new(),
    };
    decisions
        .provider_reported_cost(&response)
        .and_then(|amount| amount.to_f64())
        .and_then(Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

fn model_json(value: &Value) -> ModelJson {
    match value {
        Value::Null => ModelJson::Null,
        Value::Bool(flag) => ModelJson::Bool(*flag),
        Value::Number(number) => ModelJson::Number(number.clone()),
        Value::String(text) => ModelJson::Text(text.clone()),
        Value::Array(items) => ModelJson::List(items.iter().map(model_json).collect()),
        Value::Object(fields) => ModelJson::Object(
            fields
                .iter()
                .map(|(name, field)| (name.clone(), model_json(field)))
                .collect(),
        ),
    }
}

fn error_class(case: &ErrorCase) -> Value {
    let error = get_error_class(&case.message, case.status, case.headers.clone());
    let mut result = Map::new();
    result.insert(String::from("message"), Value::String(error.message));
    result.insert(String::from("status"), Value::from(error.status_code));
    Value::Object(result)
}
