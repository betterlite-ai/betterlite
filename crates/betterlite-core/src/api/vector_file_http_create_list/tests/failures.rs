use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{
    CallRequest, Events, Fixture, RecordingConfig, Reply, Source, Wire, call_type, caller_headers,
    file_call, file_deps, file_params, openai_config, openai_settings, params,
    request_with_headers,
};
use super::{FILE, json, run};
use crate::api::vector_file_http_create_list::error::FileTransportError;
use crate::api::vector_file_http_create_list::service::{FileDeps, Operation};
use crate::api::vector_store_http_write::error::HeaderMap;
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn create_list_failure_boundaries(#[case] mode: ClientKind) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(
        Reply::json(429, "slow down")
            .with_header("retry-after", "3")
            .with_header("content-type", "text/plain")
            .with_header("content-length", "9"),
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let files = params("http://mock.test/v1");
    let request = CallRequest::new("vs_a", &files);
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let mut fixture = Fixture::new("vector_store_file_create");
    let call = file_call(&request, Operation::Create, &original, None);
    let result = run(&mut fixture, &deps, mode, &call, &events).await;
    let Err(FileTransportError::Provider(error)) = result else {
        panic!("a 429 is a provider error");
    };
    assert_eq!(error.status_code, 429);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, "slow down");
    assert_eq!(
        error.headers,
        HeaderMap::from([
            ("content-length".to_owned(), Json::Text("9".to_owned()),),
            (
                "content-type".to_owned(),
                Json::Text("text/plain".to_owned()),
            ),
            ("retry-after".to_owned(), Json::Text("3".to_owned())),
        ])
    );
    assert_eq!(wire.requests().len(), 1);
}

#[rstest]
#[case::sync(ClientKind::Sync)]
#[case::asynchronous(ClientKind::Asynchronous)]
#[tokio::test]
async fn send_failure_becomes_a_synthesized_500(#[case] mode: ClientKind) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let files = params("http://mock.test/v1");
    let request = CallRequest::new("vs_a", &files);
    let mut fixture = Fixture::new("vector_store_file_create");
    let events = Events::new();
    let wire = Wire::failing(TransportError::Closed, &events);
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let call = file_call(&request, Operation::Create, &original, None);
    let result = run(&mut fixture, &deps, mode, &call, &events).await;
    let Err(FileTransportError::Provider(error)) = result else {
        panic!("a failed send is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert_eq!(error.message, TransportError::Closed.to_string());
    assert!(error.headers.is_empty());
    assert_eq!(wire.requests().len(), 1);
}

#[tokio::test]
async fn list_status_error_body_is_a_result() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let error_body = r#"{"error":{"message":"slow down"}}"#;
    let wire = Wire::replying(Reply::json(429, error_body), &events);
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let files = params("http://mock.test/v1");
    let request = CallRequest::new("vs_a", &files);
    let query = file_params(r#"{"limit":2}"#);
    let mut fixture = Fixture::new("vector_store_file_list");
    let call = file_call(&request, Operation::List, &query, None);
    let value = run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the list hands any status to the response transform");
    assert_eq!(value, json(error_body));
    assert_eq!(wire.requests().len(), 1);
}

#[rstest]
#[case::create(Operation::Create)]
#[case::list(Operation::List)]
#[tokio::test]
async fn malformed_reply_fails_outside_handle_error(#[case] operation: Operation) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(
        Reply::json(200, "{not json").with_header("retry-after", "3"),
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let files = params("http://mock.test/v1");
    let request = CallRequest::new("vs_a", &files);
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let query = file_params(r#"{"limit":2}"#);
    let mut fixture = Fixture::new("vector_store_file_create");
    let call = match operation {
        Operation::Create => file_call(&request, Operation::Create, &original, None),
        Operation::List => file_call(&request, Operation::List, &query, None),
    };
    let result = run(
        &mut fixture,
        &deps,
        ClientKind::Asynchronous,
        &call,
        &events,
    )
    .await;
    let Err(FileTransportError::Provider(error)) = result else {
        panic!("an unparsable reply is a provider error");
    };
    assert_eq!(error.status_code, 200);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, "vector store reply is not valid JSON");
    assert_eq!(
        error.headers.get("retry-after"),
        Some(&Json::Text("3".to_owned()))
    );
    assert_eq!(wire.requests().len(), 1);
}

// the decoder refuses a reply over its byte limit before parsing, where LiteLLM hands any size to response.json()
#[tokio::test]
async fn oversized_reply_fails_before_parsing() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let filler = "a".repeat(0x0100_0001);
    let big = format!("\"{filler}\"");
    let wire = Wire::replying(
        Reply::json(200, &big).with_header("retry-after", "3"),
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let files = params("http://mock.test/v1");
    let request = CallRequest::new("vs_a", &files);
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let mut fixture = Fixture::new("vector_store_file_create");
    let call = file_call(&request, Operation::Create, &original, None);
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(FileTransportError::Provider(error)) = result else {
        panic!("an oversized reply is a provider error");
    };
    assert_eq!(error.status_code, 200);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, "vector store reply exceeds the byte limit");
    assert_eq!(
        error.headers.get("retry-after"),
        Some(&Json::Text("3".to_owned()))
    );
    assert_eq!(wire.requests().len(), 1);
}

// B527's segment encoder refuses an id a path cannot carry before the pre-call runs
#[rstest]
#[case::empty("")]
#[case::dot(".")]
#[case::dot_dot("..")]
#[tokio::test]
async fn an_unusable_id_sends_nothing(#[case] id: &str) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let files = params("http://mock.test/v1");
    let request = CallRequest::new(id, &files);
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let mut fixture = Fixture::new("vector_store_file_create");
    let call = file_call(&request, Operation::Create, &original, None);
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(FileTransportError::Url(_)) = result else {
        panic!("an id the path cannot carry is a typed URL error");
    };
    assert!(wire.requests().is_empty());
    assert!(events.recorded().is_empty());
}

#[tokio::test]
async fn preparation_failures_send_nothing() {
    url_failures_send_nothing().await;
    header_failures_send_nothing().await;
}

async fn url_failures_send_nothing() {
    let events = Events::new();
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let files = params("http://unused.test");
    let request = CallRequest::new("vs_a", &files);
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let mut fixture = Fixture::new("vector_store_file_create");
    let call = file_call(&request, Operation::Create, &original, None);
    let missing = RecordingConfig::without_base();
    let missing_deps = file_deps(&missing, &source);
    let missing_result = run(
        &mut fixture,
        &missing_deps,
        ClientKind::Sync,
        &call,
        &events,
    )
    .await;
    assert!(matches!(missing_result, Err(FileTransportError::Url(_))));
    assert!(wire.requests().is_empty());
    assert!(events.recorded().is_empty());
    let unparsable = RecordingConfig::new("not a url");
    let unparsable_deps = file_deps(&unparsable, &source);
    let unparsable_result = run(
        &mut fixture,
        &unparsable_deps,
        ClientKind::Sync,
        &call,
        &events,
    )
    .await;
    assert!(matches!(
        unparsable_result,
        Err(FileTransportError::RequestUrl { .. })
    ));
    assert!(wire.requests().is_empty());
    assert!(events.recorded().is_empty());
}

async fn header_failures_send_nothing() {
    let events = Events::new();
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let files = params("http://unused.test");
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores");
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let bad_name = caller_headers(&[("bad name", "x")]);
    let named = request_with_headers("vs_a", &files, &bad_name);
    let mut fixture = Fixture::new("vector_store_file_create");
    let call = file_call(&named, Operation::Create, &original, None);
    let named_result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    assert!(matches!(
        named_result,
        Err(FileTransportError::HeaderName { .. })
    ));
    assert!(wire.requests().is_empty());
    assert_eq!(events.recorded(), ["log"]);
    let bad_value = caller_headers(&[("X-Trace", "bad\nvalue")]);
    let valued = request_with_headers("vs_a", &files, &bad_value);
    let valued_call = file_call(&valued, Operation::Create, &original, None);
    let valued_result = run(&mut fixture, &deps, ClientKind::Sync, &valued_call, &events).await;
    assert!(matches!(
        valued_result,
        Err(FileTransportError::HeaderValue { .. })
    ));
    assert!(wire.requests().is_empty());
    assert_eq!(events.recorded(), ["log", "log"]);
}

#[tokio::test]
async fn logging_failure_sends_nothing() {
    let events = Events::new();
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores");
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let files = params("http://unused.test");
    let request = CallRequest::new("vs_a", &files);
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let mut fixture = Fixture::new("vector_store_file_create");
    fixture.raw_only();
    let call = file_call(&request, Operation::Create, &original, None);
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    assert!(matches!(result, Err(FileTransportError::Logging(_))));
    assert_eq!(events.recorded(), ["log"]);
    assert!(wire.requests().is_empty());
}

#[rstest]
#[case::create(
    Operation::Create,
    ClientKind::Sync,
    ClientKind::Asynchronous,
    Some(false)
)]
#[case::list(Operation::List, ClientKind::Asynchronous, ClientKind::Sync, None)]
#[tokio::test]
async fn wrong_mode_clients_use_the_pool(
    #[case] operation: Operation,
    #[case] supplied_kind: ClientKind,
    #[case] called_mode: ClientKind,
    #[case] ssl_verify: Option<bool>,
) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let supplied = Wire::replying(Reply::json(200, FILE), &events);
    let pooled = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(pooled.clone());
    let mut files = params("http://mock.test/v1");
    files.ssl_verify = ssl_verify;
    let request = CallRequest::new("vs_a", &files);
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let query = file_params(r#"{"limit":2}"#);
    let deps = FileDeps {
        config: &config,
        source: &source,
        supplied: Some(SuppliedClient {
            kind: supplied_kind,
            transport: &supplied,
        }),
    };
    let mut fixture = Fixture::new(call_type(operation));
    let call = match operation {
        Operation::Create => file_call(&request, Operation::Create, &original, None),
        Operation::List => file_call(&request, Operation::List, &query, None),
    };
    run(&mut fixture, &deps, called_mode, &call, &events)
        .await
        .expect("the call succeeds on the pool");
    assert_eq!(source.calls(), [ssl_verify]);
    assert!(supplied.requests().is_empty());
    assert_eq!(pooled.requests().len(), 1);
}

#[rstest]
#[case::create(Operation::Create, ClientKind::Sync)]
#[case::list(Operation::List, ClientKind::Asynchronous)]
#[tokio::test]
async fn matching_clients_are_reused(#[case] operation: Operation, #[case] mode: ClientKind) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let supplied = Wire::replying(Reply::json(200, FILE), &events);
    let pooled = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(pooled.clone());
    let files = params("http://mock.test/v1");
    let request = CallRequest::new("vs_a", &files);
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let query = file_params(r#"{"limit":2}"#);
    let deps = FileDeps {
        config: &config,
        source: &source,
        supplied: Some(SuppliedClient {
            kind: mode,
            transport: &supplied,
        }),
    };
    let mut fixture = Fixture::new(call_type(operation));
    let call = match operation {
        Operation::Create => file_call(&request, Operation::Create, &original, None),
        Operation::List => file_call(&request, Operation::List, &query, None),
    };
    run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the supplied client serves the call");
    assert!(source.calls().is_empty());
    assert_eq!(supplied.requests().len(), 1);
    assert!(pooled.requests().is_empty());
}
