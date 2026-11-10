use std::time::Duration;

use betterlite_anthropic::count_tokens::counter::{
    AnthropicTokenCounter, CountTokensCall, TokenCountResponse, TokenCounter as _,
};
use betterlite_anthropic::count_tokens::error::ApiError;
use betterlite_anthropic::count_tokens::handler::AnthropicCountTokensHandler;
use betterlite_anthropic::count_tokens::headers::get_required_headers;
use betterlite_anthropic::count_tokens::model_info::AnthropicModelInfo;
use betterlite_anthropic::count_tokens::validate::validate_request;
use betterlite_anthropic::count_tokens::wire::{
    CountResponse, CountTokensBody, CountTokensRequest, transform_request,
};
use betterlite_models::decisions::Json;
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::{Map, Value, json};
use tokio::runtime::Runtime;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::fixture::{COUNT_HEADERS, Logs, Script, request_json, request_timeout};
use super::input::{
    Case, beta_version, credential_settings, deployment, failure, json_value, optional_params,
};

use crate::helpers::{HelperError, HelperResult};

pub(crate) fn validate(cases: &[Case]) -> HelperResult {
    let mut results = Vec::new();
    for case in cases {
        let messages = case.messages.as_ref().map_or(Ok(Json::Null), json_value)?;
        let system = case.system.as_ref().map(json_value).transpose()?;
        let tools = case.tools.as_ref().map(json_value).transpose()?;
        let body = CountTokensBody {
            model: &case.model,
            messages: &messages,
            tools: tools.as_ref(),
            system: system.as_ref(),
            optional_params: None,
        };
        results.push(match validate_request(&body) {
            Ok(()) => json!({"ok": true, "message": null}),
            Err(error) => json!({"ok": false, "message": error.to_string()}),
        });
    }
    Ok(json!({"results": results}))
}

pub(crate) fn transform(cases: &[Case]) -> HelperResult {
    let bodies = cases
        .iter()
        .map(transformed)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"bodies": bodies}))
}

fn transformed(case: &Case) -> Result<Value, Box<HelperError>> {
    let messages = case.messages.as_ref().map_or(Ok(Json::Null), json_value)?;
    let system = case.system.as_ref().map(json_value).transpose()?;
    let tools = case.tools.as_ref().map(json_value).transpose()?;
    let params = optional_params(case.optional_params.as_ref())?;
    let body = CountTokensBody {
        model: &case.model,
        messages: &messages,
        tools: tools.as_ref(),
        system: system.as_ref(),
        optional_params: params.as_ref(),
    };
    serde_json::to_value(transform_request(&body)).map_err(failure)
}

pub(crate) fn headers(cases: &[Case]) -> HelperResult {
    let beta = beta_version(&std::collections::BTreeMap::new())?;
    let mut answers = Vec::new();
    for case in cases {
        let key = SecretString::from(case.api_key.clone().unwrap_or_default());
        let map = get_required_headers(&key, &beta).map_err(failure)?;
        let mut fields = Map::new();
        for name in COUNT_HEADERS {
            fields.insert(name.to_owned(), header_value(&map, name));
        }
        answers.push(Value::Object(fields));
    }
    Ok(json!({"headers": answers}))
}

fn header_value(headers: &HeaderMap, name: &str) -> Value {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map_or(Value::Null, |found| Value::String(found.to_owned()))
}

pub(crate) fn handle(cases: &[Case], runtime: &Runtime) -> HelperResult {
    let mut answers = Vec::new();
    for case in cases {
        answers.push(handle_case(case, runtime)?);
    }
    Ok(json!({"calls": answers}))
}

fn handle_case(case: &Case, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let handler = AnthropicCountTokensHandler {
        transport: Script::new(case.reply.as_ref()),
        request_timeout: request_timeout(case.request_timeout)?,
        beta_version: beta_version(&case.environment)?,
    };
    let messages = case.messages.as_ref().map_or(Ok(Json::Null), json_value)?;
    let system = case.system.as_ref().map(json_value).transpose()?;
    let tools = case.tools.as_ref().map(json_value).transpose()?;
    let params = optional_params(case.optional_params.as_ref())?;
    let key = SecretString::from(case.api_key.clone().unwrap_or_default());
    let request = CountTokensRequest {
        body: CountTokensBody {
            model: &case.model,
            messages: &messages,
            tools: tools.as_ref(),
            system: system.as_ref(),
            optional_params: params.as_ref(),
        },
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

fn optional_duration(seconds: Option<f64>) -> Result<Option<Duration>, Box<HelperError>> {
    seconds
        .map(|found| Duration::try_from_secs_f64(found).map_err(|error| failure(&error)))
        .transpose()
}

fn response_json(outcome: Result<CountResponse, ApiError>) -> Result<Value, Box<HelperError>> {
    match outcome {
        Ok(value) => Ok(json!({"value": serde_json::to_value(value).map_err(failure)?})),
        Err(error) => Ok(json!({"error": {
            "status_code": error.status_code,
            "processing": error.message.starts_with("CountTokens processing error: "),
        }})),
    }
}

pub(crate) fn counter(cases: &[Case], runtime: &Runtime) -> HelperResult {
    let mut answers = Vec::new();
    for case in cases {
        answers.push(counter_case(case, runtime)?);
    }
    Ok(json!({"calls": answers}))
}

fn counter_case(case: &Case, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let settings = credential_settings(&case.environment)?;
    let handler = AnthropicCountTokensHandler {
        transport: Script::new(case.reply.as_ref()),
        request_timeout: request_timeout(None)?,
        beta_version: beta_version(&case.environment)?,
    };
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

pub(crate) fn factory(cases: &[Case], runtime: &Runtime) -> HelperResult {
    let mut answers = Vec::new();
    for case in cases {
        answers.push(factory_case(case, runtime)?);
    }
    Ok(json!({"cases": answers}))
}

fn factory_case(case: &Case, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let settings = credential_settings(&case.environment)?;
    let handler = AnthropicCountTokensHandler {
        transport: Script::new(case.reply.as_ref()),
        request_timeout: request_timeout(None)?,
        beta_version: beta_version(&case.environment)?,
    };
    let counter = AnthropicModelInfo { settings }.get_token_counter(handler);
    let messages = case.messages.as_ref().map_or(Ok(Json::Null), json_value)?;
    let params = deployment(case.deployment.as_ref());
    let selected: Vec<Value> = case
        .providers
        .iter()
        .map(|provider| Value::Bool(counter.should_use_token_counting_api(provider.as_deref())))
        .collect();
    let outcomes = runtime.block_on(factory_counts(&counter, case, &messages, params.as_ref()));
    let requests = counter.handler.transport.requests();
    Ok(json!({
        "selected": selected,
        "results": outcomes
            .iter()
            .map(|outcome| outcome.as_ref().map_or(Value::Null, result_json))
            .collect::<Vec<_>>(),
        "urls": requests.iter().map(|seen| Value::String(seen.url.clone())).collect::<Vec<_>>(),
    }))
}

async fn factory_counts(
    counter: &AnthropicTokenCounter<Script>,
    case: &Case,
    messages: &Json,
    deployment: Option<&betterlite_config::types::LiteLlmParams>,
) -> Vec<Option<TokenCountResponse>> {
    let mut outcomes = Vec::new();
    for model in &case.models {
        let call = CountTokensCall {
            model_to_use: model,
            messages: Some(messages),
            contents: None,
            deployment,
            request_model: "",
            tools: None,
            system: None,
        };
        outcomes.push(counter.count_tokens(&call).await);
    }
    outcomes
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
