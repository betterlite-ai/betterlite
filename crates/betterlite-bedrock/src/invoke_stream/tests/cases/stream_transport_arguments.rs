use std::sync::Arc;

use betterlite_http::error::HttpError;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::agentcore_stream::transport::HandlerKind;
use crate::invoke_stream::stream_transport;

use super::super::support::{
    BASE, BUFFERED, Fixture, Opened, TITAN, call, chunk_frame, contents, data, member, messages,
    run, sorted_pairs,
};

const MODEL: &str = "amazon.titan-text-express-v1";
const SERIALIZED: &str = r#"{"inputText": "hi", "max_tokens": 5}"#;
const FAKE_BASE: &str = "https://proxy.example.com/ai21/anthropic";
const RESPONSE_HEADERS: [(&str, &str); 2] = [
    ("content-type", "application/vnd.amazon.eventstream"),
    ("x-amzn-requestid", "req-1"),
];

pub(super) fn stream_transport_arguments() {
    carried_arguments();
    fake_stream_switch();
    chunk_size_option();
    reaches_b691();
    fake_stream_reaches_b691();
    chunk_size_reaches_b691();
}

fn carried_arguments() {
    let messages = messages();
    let data = data();
    let mut call = call(MODEL, BASE, &data, &messages);
    call.json_mode = Some(true);
    call.signed_json_body = Some(b"c2lnbmVk");
    call.control = ControlOptions::new(Some(7)).unwrap();

    let asynchronous = stream_transport(&call, false);
    assert_eq!(asynchronous.model, MODEL);
    assert_eq!(asynchronous.api_base, BASE);
    assert_eq!(
        sorted_pairs(&asynchronous.headers),
        vec![("content-type".to_owned(), "application/json".to_owned())]
    );
    assert_eq!(asynchronous.data, SERIALIZED);
    assert_eq!(asynchronous.messages, &messages);
    assert_eq!(asynchronous.json_mode, Some(true));
    assert_eq!(asynchronous.provider, Some("amazon"));
    assert!(!asynchronous.fake_stream);
    assert_eq!(asynchronous.stream_chunk_size, Some(7));
    assert!(!asynchronous.sync);
    assert_eq!(asynchronous.signed_json_body, None);

    let synchronous = stream_transport(&call, true);
    assert!(synchronous.sync);
    assert_eq!(synchronous.signed_json_body, Some(b"c2lnbmVk".as_slice()));
}

fn fake_stream_switch() {
    let messages = messages();
    let data = data();
    let proxy_call = call(MODEL, FAKE_BASE, &data, &messages);
    let proxy = stream_transport(&proxy_call, false);
    assert!(proxy.fake_stream);
    let upper_call = call(
        MODEL,
        "https://proxy.example.com/AI21/anthropic",
        &data,
        &messages,
    );
    let upper = stream_transport(&upper_call, false);
    assert!(!upper.fake_stream);
    let named_call = call("ai21.j2-mid-v1", BASE, &data, &messages);
    let named = stream_transport(&named_call, false);
    assert_eq!(named.provider, Some("ai21"));
    assert!(!named.fake_stream);
}

fn chunk_size_option() {
    let messages = messages();
    let data = data();
    let plain_call = call(MODEL, BASE, &data, &messages);
    let plain = stream_transport(&plain_call, false);
    assert_eq!(plain.stream_chunk_size, None);
}

fn reaches_b691() {
    let messages = messages();
    let data = data();
    let call = call(MODEL, BASE, &data, &messages);
    let fixture = Fixture::new(HandlerKind::Async, &[], Vec::new());
    let opened = run(
        &call,
        Some(Arc::clone(&fixture)),
        Arc::clone(&fixture),
        false,
    );
    let posts = fixture.recorded();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].url, BASE);
    assert_eq!(
        posts[0].headers,
        vec![("content-type".to_owned(), "application/json".to_owned())]
    );
    assert_eq!(posts[0].body, SERIALIZED.as_bytes().to_vec());
    assert!(posts[0].stream);
    assert_eq!(member(&opened.logs[0], "input"), &messages);
}

// fake_stream decides both the request's stream flag and whether B691 reads the reply as one buffered body.
fn fake_stream_reaches_b691() {
    let messages = messages();
    let data = data();
    let call = call(MODEL, FAKE_BASE, &data, &messages);
    let fixture = Fixture::new(
        HandlerKind::Async,
        &RESPONSE_HEADERS,
        BUFFERED.as_bytes().to_vec(),
    );
    let opened = run(
        &call,
        Some(Arc::clone(&fixture)),
        Arc::clone(&fixture),
        false,
    );
    assert_eq!(opened.failure, None);
    assert_eq!(
        contents(&opened.chunks).first(),
        Some(&Some(String::from("hello")))
    );
    assert_eq!(fixture.recorded()[0].stream, false);
}

// stream_chunk_size is applied to the response bytes: a read error drops the frame it had buffered.
fn chunk_size_reaches_b691() {
    let messages = messages();
    let data = data();
    let sized = sized_run(Some(4_096), &messages, &data);
    assert!(sized.chunks.is_empty());
    assert!(sized.failure.is_some());

    let plain = sized_run(None, &messages, &data);
    assert_eq!(
        contents(&plain.chunks).first(),
        Some(&Some(String::from("first")))
    );
    assert!(plain.failure.is_some());
}

fn sized_run(size: Option<u64>, messages: &Json, data: &IndexMap<String, Json>) -> Opened {
    let mut call = call(MODEL, BASE, data, messages);
    call.control = ControlOptions::new(size).unwrap();
    let fixture = Fixture::failing(
        HandlerKind::Async,
        &RESPONSE_HEADERS,
        vec![Ok(chunk_frame(TITAN)), Err(HttpError::Timeout)],
    );
    run(
        &call,
        Some(Arc::clone(&fixture)),
        Arc::clone(&fixture),
        false,
    )
}
