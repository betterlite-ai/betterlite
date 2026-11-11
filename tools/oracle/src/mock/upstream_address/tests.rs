#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::TcpStream;

use rstest::rstest;
use serde_json::Value;

use super::fill;
use crate::case::{BodyKind, Request, UpstreamAnswer};
use crate::http::{client, send};
use crate::mock::MockProvider;
use crate::multipart_oracle::request::fixture;
use crate::routing_mock_scripts::MockScript;

const BASE: &str = "http://127.0.0.1:4010";
const CARD: &str = r#"{"url": "http://127.0.0.1:4010/a2a"}"#;
const BINARY_ANSWER: &str = r#"{"body_kind": "bytes", "fixture": "binary-response.bin", "content_type": "application/octet-stream"}"#;
const MARKED_BINARY_ANSWER: &str = r#"{"body_kind": "bytes", "fixture": "{{upstream}}.bin", "content_type": "application/octet-stream"}"#;

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn optional_body(text: &str) -> Option<Value> {
    serde_json::from_str(text).unwrap()
}

fn upstream_answer(body: Option<Value>, events: Option<Vec<String>>) -> UpstreamAnswer {
    UpstreamAnswer {
        status: 200,
        headers: BTreeMap::new(),
        body,
        events,
    }
}

fn parsed_events(events: &[&str]) -> Option<Vec<String>> {
    (!events.is_empty()).then(|| events.iter().map(|event| (*event).to_owned()).collect())
}

#[rstest]
#[case::agent_card(r#"{"url": "{{upstream}}"}"#, r#"{"url": "http://127.0.0.1:4010"}"#, &[])]
#[case::replicate_create(
    r#"{"urls": {"get": "{{upstream}}/v1/predictions/oracle-1"}}"#,
    r#"{"urls": {"get": "http://127.0.0.1:4010/v1/predictions/oracle-1"}}"#,
    &[]
)]
#[case::array_item(
    r#"{"urls": ["{{upstream}}/one"]}"#,
    r#"{"urls": ["http://127.0.0.1:4010/one"]}"#,
    &[]
)]
#[case::other_scalars(
    r#"{"url": "{{upstream}}/one", "retries": 3, "stream": true}"#,
    r#"{"url": "http://127.0.0.1:4010/one", "retries": 3, "stream": true}"#,
    &[]
)]
#[case::events_second_string("null", "null", &["event: first", "data: {{upstream}}/two"])]
fn fill_names_the_mock_address(
    #[case] body: &str,
    #[case] expected_body: &str,
    #[case] events: &[&str],
) {
    let mut answer = upstream_answer(optional_body(body), parsed_events(events));
    fill(&mut answer, BASE);
    assert_eq!(answer.body, optional_body(expected_body));
    let expected_events = parsed_events(events).map(|collected| {
        collected
            .into_iter()
            .map(|event| event.replace("{{upstream}}", BASE))
            .collect()
    });
    assert_eq!(answer.events, expected_events);
}

#[rstest]
#[case::json_body(r#"{"url": "http://127.0.0.1:4010/a2a"}"#, &[])]
#[case::events("null", &["event: first", "data: static"])]
#[case::object_key(r#"{"{{upstream}}": "value"}"#, &[])]
fn fill_keeps_unmarked_answers(#[case] body: &str, #[case] events: &[&str]) {
    let mut answer = upstream_answer(optional_body(body), parsed_events(events));
    let before = (answer.body.clone(), answer.events.clone());
    fill(&mut answer, BASE);
    assert_eq!((answer.body, answer.events), before);
}

#[test]
fn fill_never_touches_bytes_answers() {
    let mut bytes_answer = upstream_answer(Some(value(MARKED_BINARY_ANSWER)), None);
    let before = bytes_answer.body.clone();
    fill(&mut bytes_answer, BASE);
    assert_eq!(bytes_answer.body, before);

    let mut json_answer = UpstreamAnswer {
        status: 200,
        headers: BTreeMap::from([("x-next".to_owned(), "{{upstream}}".to_owned())]),
        body: Some(value(r#"{"url": "static"}"#)),
        events: None,
    };
    let headers = json_answer.headers.clone();
    fill(&mut json_answer, BASE);
    assert_eq!(json_answer.headers, headers);
    assert_eq!(json_answer.body, Some(value(r#"{"url": "static"}"#)));
}

fn card_script() -> MockScript {
    let answer = upstream_answer(Some(value(r#"{"url": "{{upstream}}/a2a"}"#)), None);
    MockScript::try_from(&[answer][..]).unwrap()
}

fn served_url(mock: &MockProvider) -> String {
    let client = client().unwrap();
    let request = Request {
        method: "GET".to_owned(),
        path: "/.well-known/agent-card.json".to_owned(),
        headers: BTreeMap::new(),
        body: Value::Null,
        body_kind: BodyKind::Json,
    };
    let response = send(&client, &mock.base_url, &request).unwrap();
    response
        .body
        .get("url")
        .and_then(Value::as_str)
        .unwrap()
        .to_owned()
}

#[test]
fn mock_serves_its_own_address() {
    let first = MockProvider::start(card_script()).unwrap();
    let second = MockProvider::start(card_script()).unwrap();
    assert_ne!(first.base_url, second.base_url);
    let first_url = served_url(&first);
    let second_url = served_url(&second);
    assert!(first_url.starts_with(&first.base_url));
    assert!(second_url.starts_with(&second.base_url));
    assert_eq!(first_url, format!("{}/a2a", first.base_url));
    assert_eq!(second_url, format!("{}/a2a", second.base_url));
}

fn raw_reply(mock: &MockProvider) -> Vec<u8> {
    let address = mock.base_url.strip_prefix("http://").unwrap();
    let mut stream = TcpStream::connect(address).unwrap();
    write!(
        stream,
        "GET /.well-known/agent-card.json HTTP/1.1\r\nHost: mock\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut received = Vec::new();
    stream.read_to_end(&mut received).unwrap();
    received
}

#[test]
fn mock_serves_unmarked_bytes_as_written() {
    let binary = upstream_answer(Some(value(BINARY_ANSWER)), None);
    let json_answer = upstream_answer(Some(value(CARD)), None);
    let bytes_mock = MockProvider::start(MockScript::try_from(&[binary][..]).unwrap()).unwrap();
    let json_mock = MockProvider::start(MockScript::try_from(&[json_answer][..]).unwrap()).unwrap();
    assert!(raw_reply(&bytes_mock).ends_with(&fixture("binary-response.bin").unwrap()));
    assert!(raw_reply(&json_mock).ends_with(&serde_json::to_vec(&value(CARD)).unwrap()));
}
