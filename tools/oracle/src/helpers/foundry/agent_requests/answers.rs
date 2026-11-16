use std::collections::BTreeMap;

use betterlite_foundry::agent_requests::endpoint::{AgentEndpoint, endpoint_url};
use betterlite_foundry::agent_requests::error::{
    AgentRequestError, check_response, error_for_status,
};
use betterlite_foundry::agent_requests::params::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use betterlite_foundry::agent_requests::prepare::{Headers, prepare, validate_environment};
use betterlite_foundry::agent_requests::request::{
    AgentCallRequest, agent_id, complete_url, provider_info, transform_request,
};
use betterlite_foundry::agent_requests::stream::{
    HAS_CUSTOM_STREAM_WRAPPER, SHOULD_FAKE_STREAM, SUPPORTS_STREAM_PARAM_IN_BODY,
};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::input::{
    AgentIdCall, CheckCall, ErrorCall, FlagCall, ParamsCall, PathCall, PathFunction,
    PreparationCall, ProviderInfoCall, RequestCall, UrlCall, ValidationCall,
};
use super::secrets;

fn failure(error: &AgentRequestError) -> Value {
    json!({"ok": false, "error": {"class": error.class(), "status": error.status()}})
}

fn shown_headers(headers: &Headers) -> BTreeMap<&str, &str> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect()
}

pub(super) fn agent_ids(calls: &[AgentIdCall]) -> HelperResult {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        match agent_id(&call.model, &call.optional_params) {
            Ok(selected) => answers.push(json!({"ok": true, "value": selected})),
            Err(error) => answers.push(failure(&error)),
        }
    }
    Ok(json!({"calls": answers}))
}

pub(super) fn params(calls: &[ParamsCall]) -> HelperResult {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let mapped = map_openai_params(
            &call.non_default_params,
            call.optional_params.clone(),
            &call.model,
            false,
        );
        answers.push(json!({"supported": SUPPORTED_OPENAI_PARAMS, "mapped": mapped}));
    }
    Ok(json!({"calls": answers}))
}

pub(super) fn provider_infos(calls: &[ProviderInfoCall]) -> HelperResult {
    let secrets = secrets::lookup();
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let key = call.api_key.as_deref().map(SecretString::from);
        let resolved = provider_info(
            call.api_base.as_deref(),
            key.as_ref(),
            &secrets,
            &call.environment,
        );
        answers.push(match resolved {
            Ok(info) => json!({
                "ok": true,
                "value": {
                    "api_base": info.api_base,
                    "api_key": info.api_key.map(|text| text.expose_secret().to_owned()),
                },
            }),
            Err(error) => failure(&error),
        });
    }
    Ok(json!({"calls": answers}))
}

pub(super) fn complete_urls(calls: &[UrlCall]) -> HelperResult {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        answers.push(match complete_url(call.api_base.as_deref()) {
            Ok(url) => json!({"ok": true, "value": url}),
            Err(error) => failure(&error),
        });
    }
    Ok(json!({"calls": answers}))
}

pub(super) fn requests(calls: &[RequestCall]) -> HelperResult {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let headers = BTreeMap::new();
        let request = AgentCallRequest {
            model: &call.model,
            messages: &call.messages,
            optional_params: &call.optional_params,
            headers: &headers,
            api_key: None,
            api_base: None,
        };
        answers.push(match transform_request(&request) {
            Ok(payload) => {
                json!({"ok": true, "value": serde_json::to_value(&payload).or_config_error()?})
            }
            Err(error) => failure(&error),
        });
    }
    Ok(json!({"calls": answers}))
}

pub(super) fn preparations(calls: &[PreparationCall]) -> HelperResult {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let key = call.api_key.as_deref().map(SecretString::from);
        let request = AgentCallRequest {
            model: &call.model,
            messages: &[],
            optional_params: &call.optional_params,
            headers: &call.headers,
            api_key: key.as_ref(),
            api_base: Some(&call.api_base),
        };
        answers.push(match prepare(&request) {
            Ok(prepared) => json!({
                "ok": true,
                "value": {
                    "headers": shown_headers(&prepared.headers),
                    "api_version": prepared.api_version,
                    "agent_id": prepared.agent_id,
                    "thread_id": prepared.thread_id,
                    "api_base": prepared.api_base,
                },
            }),
            Err(error) => failure(&error),
        });
    }
    Ok(json!({"calls": answers}))
}

pub(super) fn validations(calls: &[ValidationCall]) -> HelperResult {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let key = call.api_key.as_deref().map(SecretString::from);
        let headers = validate_environment(&call.headers, key.as_ref());
        answers.push(json!({"ok": true, "value": {"headers": shown_headers(&headers)}}));
    }
    Ok(json!({"calls": answers}))
}

pub(super) fn checks(calls: &[CheckCall]) -> HelperResult {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        answers.push(
            match check_response(call.status, &call.body, &call.expected, &call.error_msg) {
                Ok(()) => json!({"ok": true}),
                Err(error) => failure(&error),
            },
        );
    }
    Ok(json!({"calls": answers}))
}

pub(super) fn errors(calls: &[ErrorCall]) -> HelperResult {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let error = error_for_status(call.status, call.message.clone());
        answers.push(json!({
            "class": error.class(),
            "status": error.status(),
            "headers_dropped": true,
        }));
    }
    Ok(json!({"calls": answers}))
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

pub(super) fn paths(calls: &[PathCall]) -> HelperResult {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        let url = endpoint_url(segment(call), &call.api_base, &call.api_version);
        answers.push(match url {
            Ok(url) => json!({"ok": true, "value": url}),
            Err(error) => failure(&error),
        });
    }
    Ok(json!({"calls": answers}))
}

pub(super) fn flags(calls: &[FlagCall]) -> HelperResult {
    let mut answers = Vec::with_capacity(calls.len());
    for call in calls {
        answers.push(json!({
            "model": call.model,
            "stream": call.stream,
            "should_fake_stream": SHOULD_FAKE_STREAM,
            "has_custom_stream_wrapper": HAS_CUSTOM_STREAM_WRAPPER,
            "supports_stream_param_in_request_body": SUPPORTS_STREAM_PARAM_IN_BODY,
            "transform_response_is_noop": true,
        }));
    }
    Ok(json!({"calls": answers}))
}
