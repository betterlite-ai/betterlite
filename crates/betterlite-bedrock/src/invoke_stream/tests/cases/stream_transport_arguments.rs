use std::sync::Arc;

use betterlite_models::controls::control_options::ControlOptions;

use crate::agentcore_stream::transport::HandlerKind;
use crate::invoke_stream::stream_transport;

use super::super::support::{BASE, Fixture, call, data, member, messages, run, sorted_pairs};

const MODEL: &str = "anthropic.claude-3-haiku-20240307-v1:0";
const SERIALIZED: &str = r#"{"inputText": "hi", "max_tokens": 5}"#;

pub(super) fn stream_transport_arguments() {
    carried_arguments();
    fake_stream_switch();
    chunk_size_option();
    reaches_b691();
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
    assert_eq!(asynchronous.provider, Some("anthropic"));
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
    let proxy = stream_transport(
        &call(
            "amazon.titan-text-express-v1",
            "https://proxy.example.com/ai21/anthropic",
            &data,
            &messages,
        ),
        false,
    );
    assert!(proxy.fake_stream);
    let upper = stream_transport(
        &call(
            "amazon.titan-text-express-v1",
            "https://proxy.example.com/AI21/anthropic",
            &data,
            &messages,
        ),
        false,
    );
    assert!(!upper.fake_stream);
    let named = stream_transport(&call("ai21.j2-mid-v1", BASE, &data, &messages), false);
    assert_eq!(named.provider, Some("ai21"));
    assert!(!named.fake_stream);
}

fn chunk_size_option() {
    let messages = messages();
    let data = data();
    let default = stream_transport(
        &call("amazon.titan-text-express-v1", BASE, &data, &messages),
        false,
    );
    assert_eq!(default.stream_chunk_size, None);
}

fn reaches_b691() {
    let messages = messages();
    let data = data();
    let call = call("amazon.titan-text-express-v1", BASE, &data, &messages);
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
