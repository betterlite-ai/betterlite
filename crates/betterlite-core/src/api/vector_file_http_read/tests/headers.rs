use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

use super::support::{
    Events, Fixture, Reply, Source, Wire, caller_headers, invocation, invoke, openai_config,
    openai_settings, params,
};
use super::{CONTENT_BODY, RETRIEVE_BODY, json};
use crate::api::vector_file_http_read::VectorFileReadTransport;
use crate::api::vector_file_http_read::service::{ClientKind, Operation};

const fn body(operation: Operation) -> &'static str {
    match operation {
        Operation::Retrieve => RETRIEVE_BODY,
        Operation::Content => CONTENT_BODY,
    }
}

async fn read_with_headers(
    mode: ClientKind,
    operation: Operation,
    headers: Option<&BTreeMap<String, String>>,
) -> (Wire, Fixture, Events) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, body(operation)), &events);
    let transport = VectorFileReadTransport {
        source: Source::new(wire.clone()),
    };
    let read_params = params("http://mock.test/v1");
    let mut fixture = Fixture::new("vector_store_file_retrieve", &events);
    let mut call = invocation("vs/a", "file/b", &read_params, operation, mode);
    call.headers = headers;
    let answer = invoke(&transport, &config, &mut fixture, call)
        .await
        .expect("the read succeeds");
    assert_eq!(answer, json(body(operation)));
    (wire, fixture, events)
}

fn assert_sent(wire: &Wire, wire_pairs: &[(&str, &str)]) {
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    let sent: BTreeMap<String, String> = requests[0]
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().expect("the sent header text").to_owned(),
            )
        })
        .collect();
    let expected: BTreeMap<String, String> = wire_pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    assert_eq!(sent, expected);
}

fn assert_pre_call(fixture: &Fixture, events: &Events, logged: &Json) {
    assert_eq!(fixture.logged("headers"), Some(logged));
    assert_eq!(fixture.input(), Some(&json(r#""""#)));
    assert_eq!(fixture.logged("complete_input_dict"), Some(&json("{}")));
    assert_eq!(
        fixture.logged("api_base"),
        Some(&json(r#""http://mock.test/v1/vector_stores/vs%2Fa/files""#))
    );
    assert_eq!(events.recorded(), ["log", "send"]);
}

async fn check(
    mode: ClientKind,
    operation: Operation,
    headers: Option<&BTreeMap<String, String>>,
    expected: &[(&str, &str)],
    logged: &Json,
) {
    let (wire, fixture, events) = read_with_headers(mode, operation, headers).await;
    assert_sent(&wire, expected);
    assert_pre_call(&fixture, &events, logged);
}

#[tokio::test]
async fn read_header_mutation() {
    let mutating = caller_headers(&[
        ("Authorization", "Bearer caller"),
        ("Content-Type", "text/plain"),
        ("OpenAI-Beta", "assistants=v1"),
        ("X-Trace", "t"),
    ]);
    let expected = [
        ("authorization", "Bearer sk-provider"),
        ("content-type", "application/json"),
        ("openai-beta", "assistants=v1"),
        ("x-trace", "t"),
    ];
    let logged = json(
        r#"{"Authorization":"Be****er","Content-Type":"application/json","OpenAI-Beta":"assistants=v1","X-Trace":"t"}"#,
    );
    for mode in [ClientKind::Sync, ClientKind::Asynchronous] {
        for operation in [Operation::Retrieve, Operation::Content] {
            check(mode, operation, Some(&mutating), &expected, &logged).await;
        }
    }
}

#[tokio::test]
async fn read_header_defaults() {
    let empty = BTreeMap::new();
    let expected = [
        ("authorization", "Bearer sk-provider"),
        ("content-type", "application/json"),
        ("openai-beta", "assistants=v2"),
    ];
    let logged = json(
        r#"{"Authorization":"Be****er","Content-Type":"application/json","OpenAI-Beta":"assistants=v2"}"#,
    );
    for mode in [ClientKind::Sync, ClientKind::Asynchronous] {
        for operation in [Operation::Retrieve, Operation::Content] {
            check(mode, operation, None, &expected, &logged).await;
            check(mode, operation, Some(&empty), &expected, &logged).await;
        }
    }
}
