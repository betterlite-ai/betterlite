use std::sync::Arc;

use betterlite_models::decisions::Json;

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

pub(super) fn wrapper_identity() {
    identity_of_both_wrappers();
    logging_context();
    client_chunks_and_logging();
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
    );
    assert_duration_millis(&blocking, 1_000);
    assert_eq!(blocking.timings, vec![timing(SYNC_TIMING)]);

    let asynchronous = Fixture::new(HandlerKind::Async, &RESPONSE_HEADERS, chunk_frame(TITAN));
    let streamed = run(
        &call,
        Some(Arc::clone(&asynchronous)),
        Arc::clone(&asynchronous),
        false,
    );
    assert_duration_millis(&streamed, 1_000);
    assert_eq!(streamed.timings, vec![timing(ASYNC_TIMING)]);

    let refused = Fixture::refusing(HandlerKind::Async);
    let failed = run(
        &call,
        Some(Arc::clone(&refused)),
        Arc::clone(&refused),
        false,
    );
    assert!(failed.failure.is_some());
    assert!(failed.chunks.is_empty());
    assert_duration_millis(&failed, 1_000);
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

// The fake clock starts at 0 and answers 1 for every later reading.
fn timing(call_type: &str) -> TimingRecord {
    TimingRecord {
        call_type: call_type.to_owned(),
        duration: 1.0,
        start_time: 0.0,
        end_time: 1.0,
    }
}

fn header_pairs() -> Vec<(String, String)> {
    RESPONSE_HEADERS
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}
