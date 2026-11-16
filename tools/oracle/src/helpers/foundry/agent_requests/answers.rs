use std::collections::BTreeMap;

use betterlite_foundry::agent_requests::endpoint::AgentEndpoint;
use betterlite_foundry::agent_requests::error::AgentRequestError;
use betterlite_foundry::agent_requests::prepare::{Headers, prepare};
use betterlite_foundry::agent_requests::request::{
    AgentCallRequest, provider_info, transform_request,
};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::{Value, json};

use super::input::{PathCall, PathFunction, PreparationCall, ProviderInfoCall, RequestCall};
use super::secrets;

pub(super) type CallAnswer = Result<Value, Box<HelperError>>;

pub(super) fn answer<T, F>(calls: &[T], one: F) -> HelperResult
where
    F: Fn(&T) -> CallAnswer,
{
    let answers = calls.iter().map(one).collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"calls": answers}))
}

// the answer shape the Python helper uses: ok with a value, or not ok with the error's status
pub(super) fn outcome<T: Serialize>(result: Result<T, AgentRequestError>) -> CallAnswer {
    match result {
        Ok(value) => {
            let payload = serde_json::to_value(value).or_config_error()?;
            Ok(json!({"ok": true, "value": payload}))
        }
        Err(error) => Ok(json!({"ok": false, "error": {"status": error.status()}})),
    }
}

pub(super) fn shown_headers(headers: &Headers) -> BTreeMap<&str, &str> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect()
}

pub(super) fn secret_headers(headers: &BTreeMap<String, String>) -> Headers {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
        .collect()
}

pub(super) fn resolved_provider(call: &ProviderInfoCall) -> CallAnswer {
    let key = call.api_key.as_deref().map(SecretString::from);
    let resolved = provider_info(
        call.api_base.as_deref(),
        key.as_ref(),
        &secrets::lookup(),
        &call.environment,
    );
    outcome(resolved.map(|info| {
        json!({
            "api_base": info.api_base,
            "api_key": info.api_key.map(|text| text.expose_secret().to_owned()),
        })
    }))
}

pub(super) fn transformed_request(call: &RequestCall) -> CallAnswer {
    let headers = Headers::new();
    let request = AgentCallRequest {
        model: &call.model,
        messages: &call.messages,
        optional_params: &call.optional_params,
        headers: &headers,
        api_key: None,
        api_base: None,
    };
    outcome(transform_request(&request))
}

pub(super) fn prepared_call(call: &PreparationCall) -> CallAnswer {
    let key = call.api_key.as_deref().map(SecretString::from);
    let incoming = secret_headers(&call.headers);
    let request = AgentCallRequest {
        model: &call.model,
        messages: &[],
        optional_params: &call.optional_params,
        headers: &incoming,
        api_key: key.as_ref(),
        api_base: Some(&call.api_base),
    };
    outcome(prepare(&request).map(|prepared| {
        json!({
            "headers": shown_headers(&prepared.headers),
            "api_version": prepared.api_version,
            "agent_id": prepared.agent_id,
            "thread_id": prepared.thread_id,
            "api_base": prepared.api_base,
        })
    }))
}

pub(super) fn segment(call: &PathCall) -> AgentEndpoint<'_> {
    match call.function {
        PathFunction::Threads => AgentEndpoint::Threads,
        PathFunction::Messages | PathFunction::ListMessages => AgentEndpoint::Messages {
            thread_id: call.thread_id.as_deref().unwrap_or_default(),
        },
        PathFunction::Runs => AgentEndpoint::Runs {
            thread_id: call.thread_id.as_deref().unwrap_or_default(),
        },
        PathFunction::RunStatus => AgentEndpoint::RunStatus {
            thread_id: call.thread_id.as_deref().unwrap_or_default(),
            run_id: call.run_id.as_deref().unwrap_or_default(),
        },
        PathFunction::ThreadAndRun => AgentEndpoint::ThreadAndRun,
    }
}
