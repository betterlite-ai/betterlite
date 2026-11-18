use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::StreamExt as _;

use crate::agentcore_stream::FIRST_RESPONSE;
use crate::agentcore_stream::transport::HandlerKind;
use crate::converse_events::ChunkProjection;
use crate::invoke_events::InvokeChunk;
use crate::invoke_stream::{
    CUSTOM_LLM_PROVIDER, HAS_CUSTOM_STREAM_WRAPPER, SUPPORTS_STREAM_PARAM_IN_BODY,
};

use super::super::support::{
    BASE, FRAME, Fixture, block_on, call, completion_stream, data, member, messages, run,
    sorted_pairs,
};

const MODEL: &str = "anthropic.claude-3-haiku-20240307-v1:0";
const RESPONSE_HEADERS: [(&str, &str); 2] = [
    ("content-type", "application/vnd.amazon.eventstream"),
    ("x-amzn-requestid", "req-1"),
];

pub(super) fn wrapper_identity() {
    identity_of_both_wrappers();
    logging_context();
    decoded_by_b691();
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

// The wrapper hands back B691's own decoded stream, not a second decoder's.
fn decoded_by_b691() {
    let messages = messages();
    let data = data();
    let call = call("amazon.titan-text-express-v1", BASE, &data, &messages);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let fixture = Fixture::new(
        HandlerKind::Sync,
        &RESPONSE_HEADERS,
        hex::decode(FRAME).unwrap(),
    );
    let mut chunks = completion_stream(&call, fixture, &redactor, true);
    let chunk = block_on(chunks.next()).unwrap().unwrap();
    match chunk {
        InvokeChunk::Base(projection) => match *projection {
            ChunkProjection::Generic(generic) => assert_eq!(generic.text, "first"),
            ChunkProjection::Converse(_) => panic!("a generic chunk"),
        },
        InvokeChunk::Anthropic(_) | InvokeChunk::DeepSeek(_) | InvokeChunk::OpenAiCompatible(_) => {
            panic!("the base projection")
        }
    }
}

fn config_flags() {
    assert!(HAS_CUSTOM_STREAM_WRAPPER);
    assert!(!SUPPORTS_STREAM_PARAM_IN_BODY);
}

fn header_pairs() -> Vec<(String, String)> {
    RESPONSE_HEADERS
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}
