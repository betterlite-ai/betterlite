use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::JsonValue;

use crate::agentcore_stream::FIRST_RESPONSE;
use crate::agentcore_stream::transport::HandlerKind;
use crate::invoke_stream::{
    CUSTOM_LLM_PROVIDER, HAS_CUSTOM_STREAM_WRAPPER, SUPPORTS_STREAM_PARAM_IN_BODY,
};

use super::super::support::{
    BASE, Fixture, Opened, TITAN, TimingRecord, call, chunk_frame, contents, data, member,
    messages, run, sorted_pairs,
};

const MODEL: &str = "amazon.titan-text-express-v1";
const RESPONSE_HEADERS: [(&str, &str); 2] = [
    ("content-type", "application/vnd.amazon.eventstream"),
    ("x-amzn-requestid", "req-1"),
];
const ASYNC_TIMING: &str = "get_async_custom_stream_wrapper <- track_llm_api_timing";
const SYNC_TIMING: &str = "get_sync_custom_stream_wrapper <- track_llm_api_timing";
// A Converse-shaped base event, which source.rs turns into the chunk a caller reads.
const CONVERSE: &str = r#"{"contentBlockIndex":0,"delta":{"text":"converse"}}"#;
// The same event with the trace fields a Converse chunk reports as provider specific fields.
const TRACE: &str =
    r#"{"contentBlockIndex":0,"delta":{"text":"traced"},"trace":{"promptRouting":{"source":"x"}}}"#;

pub(super) fn wrapper_identity() {
    identity_of_both_wrappers();
    logging_context();
    client_chunks_and_logging();
    converse_chunks();
    hidden_params();
    stream_options_add_a_usage_chunk();
    timing_on_success_and_failure();
    config_flags();
}

fn identity_of_both_wrappers() {
    let messages = messages();
    let data = data();
    let call = call(MODEL, BASE, &data, &messages);
    let asynchronous = Fixture::new(HandlerKind::Async, &RESPONSE_HEADERS, Vec::new());
    let opened = run(
        &call,
        Some(Arc::clone(&asynchronous)),
        Arc::clone(&asynchronous),
        false,
        None,
    );
    assert_eq!(opened.model, MODEL);
    assert_eq!(opened.custom_llm_provider, CUSTOM_LLM_PROVIDER);
    assert_eq!(sorted_pairs(&opened.headers), header_pairs());

    let synchronous = Fixture::new(HandlerKind::Sync, &RESPONSE_HEADERS, Vec::new());
    let sync_opened = run(
        &call,
        Some(Arc::clone(&synchronous)),
        Arc::clone(&synchronous),
        true,
        None,
    );
    assert_eq!(sync_opened.model, MODEL);
    assert_eq!(sync_opened.custom_llm_provider, CUSTOM_LLM_PROVIDER);
    assert_eq!(sorted_pairs(&sync_opened.headers), header_pairs());
}

fn logging_context() {
    let messages = messages();
    let data = data();
    let call = call(MODEL, BASE, &data, &messages);
    let fixture = Fixture::new(HandlerKind::Async, &RESPONSE_HEADERS, Vec::new());
    let opened = run(
        &call,
        Some(Arc::clone(&fixture)),
        Arc::clone(&fixture),
        false,
        None,
    );
    assert_eq!(opened.logs.len(), 1);
    let entry = &opened.logs[0];
    assert_eq!(member(entry, "input"), &messages);
    assert_eq!(
        member(entry, "original_response"),
        &Json::Text(FIRST_RESPONSE.to_owned())
    );
    assert_eq!(
        member(member(entry, "additional_args"), "complete_input_dict"),
        &Json::Text(r#"{"inputText": "hi", "max_tokens": 5}"#.to_owned())
    );
}

// The chunks a caller reads are B004's: the frame's text, then the finish chunk it synthesizes.
fn client_chunks_and_logging() {
    let messages = messages();
    let data = data();
    let call = call(MODEL, BASE, &data, &messages);

    let synchronous = Fixture::new(HandlerKind::Sync, &RESPONSE_HEADERS, chunk_frame(TITAN));
    let blocking = run(
        &call,
        Some(Arc::clone(&synchronous)),
        Arc::clone(&synchronous),
        true,
        None,
    );
    assert_eq!(blocking.failure, None);
    assert_eq!(
        contents(&blocking.chunks),
        vec![Some(String::from("first")), None]
    );
    let reasons = blocking
        .chunks
        .iter()
        .map(|chunk| {
            chunk
                .choices
                .first()
                .and_then(|choice| choice.finish_reason.clone())
        })
        .collect::<Vec<_>>();
    assert_eq!(reasons, vec![None, Some(String::from("stop"))]);
    assert_eq!(blocking.chunks[0].model, MODEL);
    assert_eq!(blocking.logged_chunks, 1);
    assert_eq!(blocking.successes.len(), 2);
    assert!(blocking.successes.iter().all(Option::is_some));
    assert_eq!(blocking.completions, 1);

    let asynchronous = Fixture::new(HandlerKind::Async, &RESPONSE_HEADERS, chunk_frame(TITAN));
    let streamed = run(
        &call,
        Some(Arc::clone(&asynchronous)),
        Arc::clone(&asynchronous),
        false,
        None,
    );
    assert_eq!(streamed.failure, None);
    assert_eq!(
        contents(&streamed.chunks),
        vec![Some(String::from("first")), None]
    );
    assert_eq!(streamed.logged_chunks, 0);
    assert_eq!(streamed.successes, vec![None]);
    assert_eq!(streamed.completions, 1);
}

// The Converse projection source.rs reads: the delta's text, and the trace fields it reports.
fn converse_chunks() {
    let messages = messages();
    let data = data();
    let call = call(MODEL, BASE, &data, &messages);

    let plain = Fixture::new(HandlerKind::Async, &RESPONSE_HEADERS, chunk_frame(CONVERSE));
    let opened = run(
        &call,
        Some(Arc::clone(&plain)),
        Arc::clone(&plain),
        false,
        None,
    );
    assert_eq!(opened.failure, None);
    assert_eq!(
        contents(&opened.chunks),
        vec![Some(String::from("converse")), None]
    );
    assert!(
        opened.chunks[0].provider_specific_fields.is_none(),
        "the chunk carries no provider fields"
    );

    let traced = Fixture::new(HandlerKind::Async, &RESPONSE_HEADERS, chunk_frame(TRACE));
    let traced_opened = run(
        &call,
        Some(Arc::clone(&traced)),
        Arc::clone(&traced),
        false,
        None,
    );
    assert_eq!(traced_opened.failure, None);
    assert_eq!(
        contents(&traced_opened.chunks).first(),
        Some(&Some(String::from("traced")))
    );
    let fields = traced_opened.chunks[0]
        .provider_specific_fields
        .as_ref()
        .expect("the trace fields a Converse chunk carries");
    assert!(fields.contains_key("trace"));
}

// CustomStreamWrapper.__init__ gives every chunk the model id, api_base and response headers it reports.
fn hidden_params() {
    let messages = messages();
    let data = data();
    let call = call(MODEL, BASE, &data, &messages);
    let fixture = Fixture::new(HandlerKind::Sync, &RESPONSE_HEADERS, chunk_frame(TITAN));
    let opened = run(
        &call,
        Some(Arc::clone(&fixture)),
        Arc::clone(&fixture),
        true,
        None,
    );
    let hidden = &opened.chunks[0].hidden_params;
    assert_eq!(
        hidden.get("model_id").and_then(JsonValue::as_str),
        Some("model-1")
    );
    assert_eq!(
        hidden.get("api_base").and_then(JsonValue::as_str),
        Some("https://bedrock.example.com")
    );
    let additional = hidden.get("additional_headers");
    let Some(JsonValue::Object(additional)) = additional else {
        panic!("the additional_headers object, not {additional:?}");
    };
    assert_eq!(
        additional
            .get("llm_provider-x-amzn-requestid")
            .and_then(JsonValue::as_str),
        Some("req-1")
    );
}

// include_usage ends the stream with a usage chunk instead of folding usage into the finish chunk.
fn stream_options_add_a_usage_chunk() {
    let messages = messages();
    let data = data();
    let call = call(MODEL, BASE, &data, &messages);
    let options: Json = serde_json::from_str(r#"{"include_usage":true}"#).unwrap();
    let fixture = Fixture::new(HandlerKind::Async, &RESPONSE_HEADERS, chunk_frame(TITAN));
    let opened = run(
        &call,
        Some(Arc::clone(&fixture)),
        Arc::clone(&fixture),
        false,
        Some(options),
    );
    assert_eq!(opened.failure, None);
    assert_eq!(opened.chunks.len(), 3);
    assert_eq!(
        contents(&opened.chunks),
        vec![Some(String::from("first")), None, None]
    );
}

// track_llm_api_timing records the duration and calls the service hook, on success and on a failed open.
fn timing_on_success_and_failure() {
    let messages = messages();
    let data = data();
    let call = call(MODEL, BASE, &data, &messages);

    let synchronous = Fixture::new(HandlerKind::Sync, &RESPONSE_HEADERS, chunk_frame(TITAN));
    let blocking = run(
        &call,
        Some(Arc::clone(&synchronous)),
        Arc::clone(&synchronous),
        true,
        None,
    );
    assert_duration_millis(&blocking, 3_000);
    assert_eq!(blocking.timings, vec![timing(SYNC_TIMING)]);

    let asynchronous = Fixture::new(HandlerKind::Async, &RESPONSE_HEADERS, chunk_frame(TITAN));
    let streamed = run(
        &call,
        Some(Arc::clone(&asynchronous)),
        Arc::clone(&asynchronous),
        false,
        None,
    );
    assert_duration_millis(&streamed, 3_000);
    assert_eq!(streamed.timings, vec![timing(ASYNC_TIMING)]);

    let refused = Fixture::refusing(HandlerKind::Async);
    let failed = run(
        &call,
        Some(Arc::clone(&refused)),
        Arc::clone(&refused),
        false,
        None,
    );
    assert!(failed.failure.is_some());
    assert!(failed.chunks.is_empty());
    assert_duration_millis(&failed, 3_000);
    assert_eq!(failed.timings, vec![timing(ASYNC_TIMING)]);
}

fn config_flags() {
    assert!(HAS_CUSTOM_STREAM_WRAPPER);
    assert!(!SUPPORTS_STREAM_PARAM_IN_BODY);
}

fn assert_duration_millis(opened: &Opened, millis: u64) {
    match &opened.duration {
        Some(Json::Number(number)) => assert_eq!(number.as_u64(), Some(millis)),
        other => panic!("the recorded llm_api_duration_ms, not {other:?}"),
    }
}

// The fake clock starts at two seconds and answers five for every later reading.
fn timing(call_type: &str) -> TimingRecord {
    TimingRecord {
        call_type: call_type.to_owned(),
        duration: 3.0,
        start_time: 2.0,
        end_time: 5.0,
    }
}

fn header_pairs() -> Vec<(String, String)> {
    RESPONSE_HEADERS
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}
