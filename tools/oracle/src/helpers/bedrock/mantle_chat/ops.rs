use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_bedrock::invoke_events::{DecoderClock, InvokeChunk};
use betterlite_bedrock::mantle_chat::HAS_CUSTOM_STREAM_WRAPPER;
use betterlite_bedrock::mantle_chat::environment::{chat_headers, messages_headers};
use betterlite_bedrock::mantle_chat::response::transform_response;
use betterlite_bedrock::mantle_chat::stream::model_response_iterator;
use betterlite_bedrock::mantle_chat::url::complete_url;
use betterlite_bedrock::mantle_inputs::records::MantleGlobals;
use betterlite_bedrock::request_policy::{
    BedrockRequestPolicy, Params as MetadataParams, RequestValue,
};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_models::decisions::Json as ParamJson;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use indexmap::{IndexMap, IndexSet};
use serde_json::json;
use uuid::Uuid;

use super::fixture::{Fixture, Session};
use super::input::{ChatCall, Input, MessagesCall, NativeSseCall, ResponseCall, UrlCall, UrlCases};
use super::mock::RecordingFetcher;

const LIMITS: ApiLimits = ApiLimits {
    max_json_bytes: 4096,
    max_event_bytes: 4096,
    max_items: 100,
};

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "BedrockMantleChat".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn metadata_params(call: &UrlCall) -> MetadataParams {
    let mut entries: IndexMap<String, RequestValue> = IndexMap::new();
    for (name, value) in &call.metadata {
        entries.insert(name.clone(), RequestValue::Text(value.clone()));
    }
    let mut params = MetadataParams::new();
    if !entries.is_empty() {
        params.insert("metadata".to_owned(), RequestValue::Mapping(entries));
    }
    if let Some(project_id) = &call.project_id {
        params.insert(
            "aws_bedrock_project_id".to_owned(),
            RequestValue::Text(project_id.clone()),
        );
    }
    params
}

fn globals(environment: &BTreeMap<String, String>) -> MantleGlobals {
    MantleGlobals {
        mantle_api_key: None,
        mantle_api_base: environment.get("BEDROCK_MANTLE_API_BASE").cloned(),
        mantle_region: None,
        aws_bearer_token: None,
        aws_region_name: environment.get("AWS_REGION_NAME").cloned(),
        aws_region: environment.get("AWS_REGION").cloned(),
    }
}

fn policy(call: &UrlCall) -> Option<BedrockRequestPolicy> {
    let fields: IndexSet<String> = call.policy_fields.iter().flatten().cloned().collect();
    (!fields.is_empty()).then(|| BedrockRequestPolicy { fields })
}

fn chat_url(call: &UrlCall) -> HelperResult {
    let mut params = call.invoke_params().map_err(failure)?;
    let headers = chat_headers(
        &call.headers,
        &mut params,
        &metadata_params(call),
        policy(call).as_ref(),
    )
    .map_err(failure)?;
    let url = complete_url(
        call.api_base.as_deref(),
        &call.model,
        &params,
        &globals(&call.environment),
    )
    .map_err(failure)?;
    Ok(json!({"url": url, "headers": headers}))
}

fn message_url(call: &UrlCall) -> HelperResult {
    let params = call.invoke_params().map_err(failure)?;
    let (headers, api_base) = messages_headers(
        &call.headers,
        &metadata_params(call),
        policy(call).as_ref(),
        call.api_base.as_deref(),
        None,
    )
    .map_err(failure)?;
    let url = complete_url(
        call.api_base.as_deref(),
        &call.model,
        &params,
        &globals(&call.environment),
    )
    .map_err(failure)?;
    Ok(json!({"url": url, "headers": headers, "api_base": api_base}))
}

fn chat_urls(argument: &UrlCases) -> HelperResult {
    let mut answers = Vec::with_capacity(argument.calls.len());
    for entry in &argument.calls {
        answers.push(chat_url(entry)?);
    }
    Ok(json!({"calls": answers}))
}

fn message_urls(argument: &UrlCases) -> HelperResult {
    let mut answers = Vec::with_capacity(argument.calls.len());
    for entry in &argument.calls {
        answers.push(message_url(entry)?);
    }
    Ok(json!({"calls": answers}))
}

fn messages_request(call: &MessagesCall) -> HelperResult {
    let fixture = Fixture::load(&call.model_cost).map_err(failure)?;
    let mut session = Session::new(&fixture).map_err(failure)?;
    let mut headers = call.request_headers.clone();
    let body = session
        .messages_transform(call, &mut headers)
        .map_err(failure)?;
    Ok(json!({"body": body, "headers": headers}))
}

fn chat(call: &ChatCall, inline: bool) -> HelperResult {
    let fixture = Fixture::load(&call.model_cost).map_err(failure)?;
    let mut session = Session::new(&fixture).map_err(failure)?;
    let fetcher = RecordingFetcher::new(&call.fetch);
    let body = session
        .chat_transform(call, &fetcher, inline)
        .map_err(failure)?;
    Ok(json!({"body": body, "fetches": fetcher.calls()}))
}

fn native_sse(call: &NativeSseCall) -> HelperResult {
    let clock = DecoderClock {
        next_id: fixed_id,
        now: fixed_now,
    };
    let mut decoder = model_response_iterator("claude", false, None, clock);
    let mut chunks = Vec::with_capacity(call.events.len());
    for event in &call.events {
        let parsed: ParamJson = serde_json::from_str(event).map_err(failure)?;
        let decoded = decoder.chunk_parser(&parsed).map_err(failure)?;
        let InvokeChunk::Anthropic(parsed_event) = decoded else {
            return Err(failure("the decoder left the Anthropic projection"));
        };
        let choice = parsed_event
            .choices
            .first()
            .ok_or_else(|| failure("the chunk has no choice"))?;
        chunks.push(json!({
            "content": choice.delta.content,
            "finish_reason": choice.finish_reason,
        }));
    }
    Ok(json!({
        "has_custom_stream_wrapper": HAS_CUSTOM_STREAM_WRAPPER,
        "chunks": chunks,
    }))
}

fn messages_response(call: &ResponseCall) -> HelperResult {
    let fields = transform_response(call.body.as_bytes(), call.status, &LIMITS).map_err(failure)?;
    Ok(json!({"response": fields}))
}

fn fixed_id() -> Uuid {
    Uuid::from_u128(0x0123_4567_89ab_4def_8123_4567_89ab_cdef)
}

const fn fixed_now() -> i64 {
    1_700_000_000
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::ChatUrl(call) => chat_urls(call),
        Input::MessageUrl(call) => message_urls(call),
        Input::MessagesRequest(call) => messages_request(call),
        Input::ChatRequest(call) => chat(call, false),
        Input::NativeSse(call) => native_sse(call),
        Input::MessagesResponse(call) => messages_response(call),
        Input::RemoteUrlInlined(call) => chat(call, true),
    }
}
