use betterlite_models::decisions::ir::{DecisionResponse, DecisionUsage};
use betterlite_models::decisions::{Extra, Json as ModelJson, TokenCount};
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig;
use betterlite_openai::openrouter_decisions::OpenRouterDecisionsConfig;
use betterlite_openai::openrouter_embedding::{
    SUPPORTED_OPENAI_PARAMS, get_complete_url as embedding_url, map_openai_params,
    transform_embedding_request, validate_environment as embedding_headers,
};
use betterlite_openai::openrouter_responses::{
    PROVIDER, SUPPORTS_NATIVE_WEBSOCKET, get_complete_url as responses_url,
    validate_environment as responses_headers,
};
use betterlite_prompts::arguments::json::Object;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::SecretString;
use serde_json::{Number, Value, json};

use crate::helpers::{HelperResult, OrHelperError as _};

use super::chat::{chat_params, chunks, config, error_class, prompt_object};
use super::input::{DecisionsCase, EmbeddingCase, Input, ResponsesCase};
use super::wire::{embedding_replies, requests, responses as chat_responses};

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Supported { models } => Ok(json!(
            models
                .iter()
                .map(|model| config().supported_params(model))
                .collect::<Vec<_>>()
        )),
        Input::ChatParams { cases } => chat_params(cases),
        Input::Chunk { cases } => Ok(chunks(cases)),
        Input::Embedding { cases } => embedding(cases),
        Input::Responses { cases } => responses(cases),
        Input::Decisions { cases } => Ok(decisions(cases)),
        Input::ErrorClass { cases } => Ok(Value::Array(cases.iter().map(error_class).collect())),
        Input::Request { cases } => requests(cases),
        Input::Response { cases } => chat_responses(cases),
        Input::EmbeddingReply { cases } => embedding_replies(cases),
    }
}

fn embedding(cases: &[EmbeddingCase]) -> HelperResult {
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let key = case.api_key.clone().map(SecretString::from);
        let headers = embedding_headers(
            case.headers.clone(),
            key.as_ref(),
            case.environment.get("OR_SITE_URL").map(String::as_str),
            case.environment.get("OR_APP_NAME").map(String::as_str),
        );
        let request = transform_embedding_request(
            &case.model,
            &case.input,
            &prompt_object(&case.optional_params),
        );
        let mut params = Object::new();
        map_openai_params(&prompt_object(&case.non_default_params), &mut params);
        results.push(json!({
            "supported": SUPPORTED_OPENAI_PARAMS,
            "headers": headers,
            "url": embedding_url(case.api_base.as_deref()),
            "request": serde_json::to_value(&request).or_value_error()?,
            "params": serde_json::to_value(&params).or_value_error()?,
        }));
    }
    Ok(Value::Array(results))
}

fn responses(cases: &[ResponsesCase]) -> HelperResult {
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let headers = responses_headers(
            case.headers.clone(),
            case.api_key.as_deref(),
            case.global_api_key.as_deref(),
            &case.environment,
        )
        .or_value_error()?;
        results.push(json!({
            "headers": headers,
            "url": responses_url(
                case.api_base.as_deref(),
                case.global_api_base.as_deref(),
                &case.environment,
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
