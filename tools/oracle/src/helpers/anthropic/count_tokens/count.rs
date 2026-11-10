use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_anthropic::count_tokens::counter::{
    AnthropicTokenCounter, CountTokensCall, TokenCountResponse, TokenCounter as _,
};
use betterlite_anthropic::count_tokens::error::ApiError;
use betterlite_anthropic::count_tokens::handler::AnthropicCountTokensHandler;
use betterlite_anthropic::count_tokens::headers::get_required_headers;
use betterlite_anthropic::count_tokens::model_info::AnthropicModelInfo;
use betterlite_anthropic::count_tokens::validate::validate_request;
use betterlite_anthropic::count_tokens::wire::{CountResponse, CountTokensRequest};
use betterlite_models::decisions::Json;
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::{Map, Value, json};
use tokio::runtime::Runtime;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::fixture::{COUNT_HEADERS, Logs, Script, request_json, request_timeout};
use super::input::{
    Case, Parts, beta_version, credential_settings, deployment, failure, json_value,
};

use crate::helpers::{HelperError, HelperResult};

pub(crate) fn answers<F>(key: &str, cases: &[Case], answer: F) -> HelperResult
where
    F: Fn(&Case) -> Result<Value, Box<HelperError>>,
{
    let mut found = Vec::new();
    for case in cases {
        found.push(answer(case)?);
    }
    Ok(json!({ key: found }))
}

pub(crate) fn validate_case(case: &Case) -> Result<Value, Box<HelperError>> {
    let parts = Parts::of(case)?;
    Ok(match validate_request(&parts.body(&case.model)) {
        Ok(()) => json!({"ok": true, "message": null}),
        Err(error) => json!({"ok": false, "message": error.to_string()}),
    })
}

pub(crate) fn headers_case(case: &Case) -> Result<Value, Box<HelperError>> {
    let key = SecretString::from(case.api_key.clone().unwrap_or_default());
    let map = get_required_headers(&key, &beta_version(&BTreeMap::new())?).map_err(failure)?;
    let fields: Map<String, Value> = COUNT_HEADERS
        .iter()
        .map(|name| ((*name).to_owned(), header_value(&map, name)))
        .collect();
    Ok(Value::Object(fields))
}

fn header_value(headers: &HeaderMap, name: &str) -> Value {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map_or(Value::Null, |found| Value::String(found.to_owned()))
}

pub(crate) fn handle_case(case: &Case, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let handler = scripted_handler(case, case.request_timeout)?;
    let parts = Parts::of(case)?;
    let key = SecretString::from(case.api_key.clone().unwrap_or_default());
    let request = CountTokensRequest {
        body: parts.body(&case.model),
        api_key: &key,
        api_base: case.api_base.as_deref(),
        timeout: optional_duration(case.timeout)?,
    };
    let outcome = runtime.block_on(handler.handle_count_tokens_request(&request));
    let sent = handler.transport.requests();
    Ok(json!({
        "request": sent.first().map(request_json),
        "response": response_json(outcome)?,
    }))
}

fn scripted_handler(
    case: &Case,
    timeout: Option<f64>,
) -> Result<AnthropicCountTokensHandler<Script>, Box<HelperError>> {
    Ok(AnthropicCountTokensHandler {
        transport: Script::new(case.reply.as_ref()),
        request_timeout: request_timeout(timeout)?,
        beta_version: beta_version(&case.environment)?,
    })
}

fn optional_duration(seconds: Option<f64>) -> Result<Option<Duration>, Box<HelperError>> {
    seconds
        .map(|found| Duration::try_from_secs_f64(found).map_err(|error| failure(&error)))
        .transpose()
}

fn response_json(outcome: Result<CountResponse, ApiError>) -> Result<Value, Box<HelperError>> {
    match outcome {
        Ok(value) => Ok(json!({"value": serde_json::to_value(value).map_err(failure)?})),
        Err(error) => Ok(json!({"error": {
            "status_code": error.status_code(),
            "processing": matches!(error, ApiError::Processing { .. }),
        }})),
    }
}

pub(crate) fn counter_case(case: &Case, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let settings = credential_settings(&case.environment)?;
    let handler = scripted_handler(case, None)?;
    let counter = AnthropicTokenCounter {
        handler,
        environment_api_key: settings.api_key,
    };
    let messages = case.messages.as_ref().map(json_value).transpose()?;
    let contents = case.contents.as_ref().map(json_value).transpose()?;
    let tools = case.tools.as_ref().map(json_value).transpose()?;
    let system = case.system.as_ref().map(json_value).transpose()?;
    let params = deployment(case.deployment.as_ref());
    let call = CountTokensCall {
        model_to_use: &case.model,
        messages: messages.as_ref(),
        contents: contents.as_ref(),
        deployment: params.as_ref(),
        request_model: case.request_model.as_deref().unwrap_or(""),
        tools: tools.as_ref(),
        system: system.as_ref(),
    };
    let selected = counter.should_use_token_counting_api(case.provider.as_deref());
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let outcome = with_default(subscriber, || runtime.block_on(counter.count_tokens(&call)));
    let sent = counter.handler.transport.requests();
    Ok(json!({
        "selected": selected,
        "result": outcome.map_or(Value::Null, |found| result_json(&found)),
        "request": sent.first().map(request_json),
        "warnings": logs.entries(),
    }))
}

pub(crate) fn factory_case(case: &Case, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let settings = credential_settings(&case.environment)?;
    let handler = scripted_handler(case, None)?;
    let counter = AnthropicModelInfo { settings }.get_token_counter(handler);
    let messages = case.messages.as_ref().map_or(Ok(Json::Null), json_value)?;
    let params = deployment(case.deployment.as_ref());
    let mut results = Vec::new();
    for model in &case.models {
        let call = CountTokensCall {
            model_to_use: model,
            messages: Some(&messages),
            contents: None,
            deployment: params.as_ref(),
            request_model: "",
            tools: None,
            system: None,
        };
        let outcome = runtime.block_on(counter.count_tokens(&call));
        results.push(outcome.map_or(Value::Null, |found| result_json(&found)));
    }
    let selected: Vec<bool> = case
        .providers
        .iter()
        .map(|provider| counter.should_use_token_counting_api(provider.as_deref()))
        .collect();
    let urls: Vec<String> = counter
        .handler
        .transport
        .requests()
        .into_iter()
        .map(|seen| seen.url)
        .collect();
    Ok(json!({"selected": selected, "results": results, "urls": urls}))
}

fn result_json(found: &TokenCountResponse) -> Value {
    json!({
        "total_tokens": found.total_tokens,
        "request_model": found.request_model,
        "model_used": found.model_used,
        "tokenizer_type": found.tokenizer_type,
        "original_response": found.original_response,
        "error": found.error,
        "error_message": error_message_kind(found),
        "status_code": found.status_code,
    })
}

fn error_message_kind(found: &TokenCountResponse) -> Option<String> {
    let message = found.error_message.as_deref()?;
    if found.status_code != Some(500) {
        return Some(message.to_owned());
    }
    let kind = if message.starts_with("CountTokens processing error: ") {
        "processing"
    } else {
        "generic"
    };
    Some(kind.to_owned())
}
