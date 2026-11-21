use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::support::{
    CallRequest, Events, Fixture, Reply, Source, Wire, caller_headers, file_call, file_deps,
    file_params, openai_config, openai_settings, request_header,
};
use super::{FILE, LIST, json, run};
use crate::api::vector_file_http_create_list::config::FilesParams;
use crate::api::vector_file_http_create_list::service::{FileRequest, Operation};
use crate::api::vector_store_http_write::service::ClientKind;

#[derive(Clone, Copy, Debug)]
enum CallerHeaders {
    Missing,
    Empty,
}

fn provider_params(api_base: &str) -> FilesParams {
    FilesParams {
        api_base: Some(api_base.to_owned()),
        api_key: Some(SecretString::from("sk-provider")),
        ssl_verify: None,
    }
}

fn assert_create_logging(fixture: &Fixture) {
    assert_eq!(fixture.input(), Some(&json(r#""""#)));
    assert_eq!(
        fixture.logged("api_base"),
        Some(&json(r#""http://mock.test/v1/vector_stores/vs_a/files""#))
    );
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json(r#"{"file_id":"file_a"}"#))
    );
    assert_eq!(
        fixture.logged("headers"),
        Some(&json(
            r#"{"Authorization":"Be****er","Content-Type":"application/json","OpenAI-Beta":"assistants=v1","X-Trace":"x"}"#
        ))
    );
}

fn assert_list_credentials(requests: &[FileRequest]) {
    assert_eq!(requests.len(), 1);
    assert_eq!(
        request_header(&requests[0], "openai-beta"),
        Some("assistants=v2")
    );
    assert_eq!(
        request_header(&requests[0], "authorization"),
        Some("Bearer sk-provider")
    );
}

fn assert_wire_headers(requests: &[FileRequest]) {
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|sent| {
        request_header(sent, "authorization") == Some("Bearer sk-provider")
            && request_header(sent, "content-type") == Some("application/json")
            && request_header(sent, "openai-beta") == Some("assistants=v1")
            && request_header(sent, "x-trace") == Some("x")
    }));
}

#[tokio::test]
async fn create_list_credentials() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-env");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let files = provider_params("http://mock.test/v1");
    let headers = caller_headers(&[
        ("Authorization", "Bearer sk-caller"),
        ("Content-Type", "text/plain"),
        ("OpenAI-Beta", "assistants=v1"),
        ("X-Trace", "x"),
    ]);
    let request = CallRequest {
        headers: Some(&headers),
        ..CallRequest::new("vs_a", &files)
    };
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let mut fixture = Fixture::new("vector_store_file_create");
    let call = file_call(&request, Operation::Create, &original, None);
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the create succeeds");
    assert_create_logging(&fixture);
    let query = file_params(r#"{"limit":4}"#);
    let list = file_call(&request, Operation::List, &query, None);
    run(&mut fixture, &deps, ClientKind::Sync, &list, &events)
        .await
        .expect("the list succeeds");
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json(r#"{"limit":4}"#))
    );
    assert_eq!(events.recorded(), ["log", "send", "log", "send"]);
    assert_wire_headers(&wire.requests());
}

#[rstest]
#[case::no_caller_headers(CallerHeaders::Missing)]
#[case::empty_caller_headers(CallerHeaders::Empty)]
#[tokio::test]
async fn list_without_caller_headers_adds_v2_beta(#[case] caller: CallerHeaders) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-env");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, LIST), &events);
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let files = provider_params("http://mock.test/v1");
    let plain = CallRequest::new("vs_a", &files);
    let empty: BTreeMap<String, String> = BTreeMap::new();
    let blank = CallRequest {
        headers: Some(&empty),
        ..CallRequest::new("vs_a", &files)
    };
    let request = match caller {
        CallerHeaders::Missing => &plain,
        CallerHeaders::Empty => &blank,
    };
    let query: IndexMap<String, Json> = IndexMap::new();
    let original = file_params("{}");
    let mut fixture = Fixture::new("vector_store_file_list");
    let call = file_call(request, Operation::List, &original, Some(&query));
    run(
        &mut fixture,
        &deps,
        ClientKind::Asynchronous,
        &call,
        &events,
    )
    .await
    .expect("the list succeeds");
    assert_list_credentials(&wire.requests());
}
