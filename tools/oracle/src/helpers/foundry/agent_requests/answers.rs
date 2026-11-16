use std::collections::BTreeMap;

use betterlite_foundry::agent_requests::endpoint::{AgentEndpoint, endpoint_url};
use betterlite_foundry::agent_requests::error::{AgentRequestError, check_response};
use betterlite_foundry::agent_requests::params::SUPPORTED_OPENAI_PARAMS;
use betterlite_foundry::agent_requests::prepare::{Headers, prepare, validate_environment};
use betterlite_foundry::agent_requests::request::{
    AgentCallRequest, agent_id, complete_url, provider_info, transform_request,
};
use betterlite_foundry::agent_requests::stream::{
    HAS_CUSTOM_STREAM_WRAPPER, SHOULD_FAKE_STREAM, SUPPORTS_STREAM_PARAM_IN_BODY,
};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::{Value, json};

use super::input::{
    CheckCall, ErrorCall, FlagCall, Input, ParamsCall, PathCall, PathFunction, PreparationCall,
    ProviderInfoCall, RequestCall, ValidationCall,
};
use super::secrets;

pub(super) type CallAnswer = Result<Value, Box<HelperError>>;

pub(super) type Calls = Result<Vec<Value>, Box<HelperError>>;

pub(super) fn run_calls(input: &Input) -> Calls {
    match input {
        Input::AgentIds(calls) => collect(calls, |call| {
            outcome(agent_id(&call.model, &call.optional_params))
        }),
        Input::Params(calls) => collect(calls, params_answer),
        Input::ProviderInfos(calls) => collect(calls, resolved_provider),
        Input::CompleteUrls(calls) => collect(calls, |call| {
            outcome(complete_url(call.api_base.as_deref()))
        }),
        Input::Requests(calls) => collect(calls, transformed_request),
        Input::Preparations(calls) => collect(calls, prepared_call),
        Input::Validations(calls) => collect(calls, validated),
        Input::Checks(calls) => collect(calls, checked),
        Input::Errors(calls) => collect(calls, error_class),
        Input::Paths(calls) => collect(calls, |call| {
            outcome(endpoint_url(
                segment(call),
                &call.api_base,
                &call.api_version,
            ))
        }),
        Input::Flags(calls) => collect(calls, flag_answer),
    }
}

fn collect<T, Answer>(calls: &[T], answer: Answer) -> Calls
where
    Answer: Fn(&T) -> CallAnswer,
{
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        answers.push(answer(call)?);
    }
    Ok(answers)
}

// the answer shape the Python helper uses: ok with a value, or not ok with the error's status
pub(super) fn outcome<T>(result: Result<T, AgentRequestError>) -> CallAnswer
where
    T: Serialize,
{
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

fn params_answer(call: &ParamsCall) -> CallAnswer {
    Ok(json!({
        "supported": SUPPORTED_OPENAI_PARAMS,
        "mapped": call.optional_params,
    }))
}

fn resolved_provider(call: &ProviderInfoCall) -> CallAnswer {
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

fn transformed_request(call: &RequestCall) -> CallAnswer {
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

fn prepared_call(call: &PreparationCall) -> CallAnswer {
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

fn validated(call: &ValidationCall) -> CallAnswer {
    let key = call.api_key.as_deref().map(SecretString::from);
    let headers = validate_environment(&secret_headers(&call.headers), key.as_ref());
    Ok(json!({"ok": true, "value": {"headers": shown_headers(&headers)}}))
}

fn checked(call: &CheckCall) -> CallAnswer {
    match check_response(call.status, &call.body, &call.expected, &call.error_msg) {
        Ok(()) => Ok(json!({"ok": true})),
        Err(error) => outcome(Err::<Value, _>(error)),
    }
}

fn error_class(call: &ErrorCall) -> CallAnswer {
    let error = AgentRequestError::Upstream {
        status: call.status,
        message: call.message.clone(),
    };
    Ok(json!({"status": error.status(), "headers_dropped": true}))
}

fn flag_answer(call: &FlagCall) -> CallAnswer {
    Ok(json!({
        "model": call.model,
        "stream": call.stream,
        "should_fake_stream": SHOULD_FAKE_STREAM,
        "has_custom_stream_wrapper": HAS_CUSTOM_STREAM_WRAPPER,
        "supports_stream_param_in_request_body": SUPPORTS_STREAM_PARAM_IN_BODY,
        "transform_response_is_noop": true,
    }))
}

fn segment(call: &PathCall) -> AgentEndpoint<'_> {
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
