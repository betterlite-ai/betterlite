use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use super::args::{AdditionalArgs, RawBody};
use super::classifier::{classifier_input_snapshot, is_classifier_call};
use super::debug_log::print_llm_call_debugging_log;
use super::env::PrePostEnv;
use super::error::PrePostError;
use super::hooks::{Phase, run_input_callbacks};
use super::masking::{mask_api_base, masked_headers};
use super::request::{CurlRequest, curl_command, raw_request_body};
use crate::exporter_options::redaction::REDACTED_TEXT;
use crate::logging_state::LoggingState;

#[derive(Deserialize)]
pub struct PreCall {
    pub input: Json,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub additional_args: AdditionalArgs,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PreCallOutput {
    pub classifier_input: Option<IndexMap<String, Json>>,
}

fn set_litellm_param(state: &mut LoggingState, name: &str, value: Json) {
    state.litellm_params.insert(name.to_owned(), value.clone());
    if let Some(Json::Object(params)) = state.model_call_details.get_mut("litellm_params") {
        params.insert(name.to_owned(), value);
    }
}

fn record_call(state: &mut LoggingState, call: &PreCall, env: &PrePostEnv<'_>) -> PreCallOutput {
    let args = &call.additional_args;
    state
        .model_call_details
        .insert("input".to_owned(), call.input.clone());
    state
        .model_call_details
        .insert("additional_args".to_owned(), args.recorded());
    state.model_call_details.insert(
        "log_event_type".to_owned(),
        Json::Text("pre_api_call".to_owned()),
    );
    let mut output = PreCallOutput::default();
    if is_classifier_call(&state.call_type, &state.litellm_params) && !env.decision.content {
        output.classifier_input = args
            .complete_input_dict
            .as_ref()
            .and_then(|body| classifier_input_snapshot(body, args.openai_sdk == Some(true)));
    }
    if let Some(model) = call.model.as_ref().filter(|name| !name.is_empty()) {
        state
            .model_call_details
            .insert("model".to_owned(), Json::Text(model.clone()));
    }
    let masked = mask_api_base(args.api_base.as_deref().unwrap_or_default());
    set_litellm_param(state, "api_base", Json::Text(masked));
    output
}

// LiteLLM keeps the raw request in the call metadata only when that dict is already non-empty
fn store_raw_request(state: &mut LoggingState, raw_request: &str) {
    if let Some(Json::Object(metadata)) = state.litellm_params.get("metadata")
        && !metadata.is_empty()
    {
        let mut updated = metadata.clone();
        updated.insert("raw_request".to_owned(), Json::Text(raw_request.to_owned()));
        set_litellm_param(state, "metadata", Json::Object(updated));
    }
}

fn record_raw_request(state: &mut LoggingState, args: &AdditionalArgs, env: &PrePostEnv<'_>) {
    let empty_body = RawBody::default();
    let empty_headers = IndexMap::new();
    let body = args.complete_input_dict.as_ref().unwrap_or(&empty_body);
    let headers = args.headers.as_ref().unwrap_or(&empty_headers);
    let api_base = args.api_base.as_deref().unwrap_or_default();
    let (shown_body, raw_request) = if env.decision.content {
        (
            Json::Text(REDACTED_TEXT.to_owned()),
            REDACTED_TEXT.to_owned(),
        )
    } else {
        let command = curl_command(&CurlRequest {
            api_base,
            headers,
            request_str: args.request_str.as_deref(),
            data: body,
        });
        (
            raw_request_body(Some(body)),
            env.redactor.redact_string(&command),
        )
    };
    let typed = IndexMap::from([
        (
            "raw_request_api_base".to_owned(),
            Json::Text(mask_api_base(api_base)),
        ),
        ("raw_request_body".to_owned(), shown_body),
        (
            "raw_request_headers".to_owned(),
            Json::Object(masked_headers(headers, false)),
        ),
        ("error".to_owned(), Json::Null),
    ]);
    state
        .model_call_details
        .insert("raw_request_typed_dict".to_owned(), Json::Object(typed));
    store_raw_request(state, &raw_request);
}

pub fn pre_call(
    state: &mut LoggingState,
    call: &PreCall,
    env: &PrePostEnv<'_>,
) -> Result<PreCallOutput, PrePostError> {
    let output = record_call(state, call, env);
    print_llm_call_debugging_log(state.litellm_request_debug, &call.additional_args, env);
    if state.request_logging.log_raw_request_response || env.globals.log_raw_request_response {
        record_raw_request(state, &call.additional_args, env);
    }
    env.run_logger_fn(&state.model_call_details);
    let started = Json::Text(env.now.to_rfc3339());
    state
        .model_call_details
        .insert("api_call_start_time".to_owned(), started.clone());
    // set once, so a retry never moves the first provider handoff
    if matches!(
        state.model_call_details.get("first_api_call_start_time"),
        None | Some(Json::Null)
    ) {
        state
            .model_call_details
            .insert("first_api_call_start_time".to_owned(), started);
    }
    run_input_callbacks(state, Phase::Pre, env);
    if state.request_logging.raw_request_only {
        return Err(PrePostError::RawRequestCaptured);
    }
    Ok(output)
}
