use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use rstest::rstest;

use super::json;
use super::run;
use super::support::{
    Events, Fixture, IMPATIENT, ProxiedSource, RecordingConfig, Reply, Source, Wire,
    caller_headers, delete_call, openai_config, openai_settings, params, target, update_call,
    update_request,
};
use crate::api::vector_file_http_write::error::FileWriteError;
use crate::api::vector_file_http_write::service::WriteDeps;
use crate::api::vector_store_http_write::error::{BaseLlmException, HeaderMap};
use crate::api::vector_store_http_write::service::{ClientKind, Operation};

const FILE: &str = r#"{"id":"file_a","object":"vector_store.file"}"#;

// A status of 400 or more becomes _handle_error's provider error: the reply's text, its masked headers and its status, with no retry
#[rstest]
#[case::update(Operation::Update)]
#[case::delete(Operation::Delete)]
#[tokio::test]
async fn file_write_failure_scope(#[case] operation: Operation) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(
        Reply::json(403, "denied")
            .with_header("x-request-id", "r")
            .with_header("content-type", "text/plain"),
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://mock.test/v1");
    let request = update_request(r#"{"attributes":{}}"#);
    let update = update_call(
        target("vs_a", "file_a", &params, None, None),
        &request,
        None,
    );
    let delete = delete_call(target("vs_a", "file_a", &params, None, None));
    let call = match operation {
        Operation::Update => &update,
        Operation::Delete => &delete,
    };
    let mut fixture = Fixture::new("vector_store_file_write");
    let result = run(&mut fixture, &deps, ClientKind::Sync, call, &events).await;
    let Err(FileWriteError::Provider(error)) = result else {
        panic!("a 403 is a provider error");
    };
    assert_eq!(error.status_code, 403);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, "denied");
    assert_eq!(error.headers.get("x-request-id"), Some(&json(r#""r""#)));
    assert_eq!(wire.requests().len(), 1, "the handler never retries");
}

// raise_for_status rejects a 400 inside the send, so the first failing status never reaches the response transform
#[tokio::test]
async fn status_400_is_a_provider_error() {
    let config = RecordingConfig::new("http://mock.test/v1");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(400, "{}"), &events);
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://unused.test");
    let request = update_request(r#"{"attributes":{}}"#);
    let call = update_call(
        target("vs_a", "file_a", &params, None, None),
        &request,
        None,
    );
    let mut fixture = Fixture::new("vector_store_file_update");
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(FileWriteError::Provider(error)) = result else {
        panic!("a 400 is a provider error, not a transformed reply");
    };
    assert_eq!(error.status_code, 400);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(wire.requests().len(), 1);
}

// The response transform runs after the send catch: its own error class answers at the reply's status and sees the raw reply headers
#[rstest]
#[case::update(Operation::Update)]
#[case::delete(Operation::Delete)]
#[tokio::test]
async fn malformed_reply_is_the_transforms_error(#[case] operation: Operation) {
    let config = RecordingConfig::new("http://mock.test/v1");
    let events = Events::new();
    let wire = Wire::replying(
        Reply::json(200, "{not json").with_header("retry-after", "3"),
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://unused.test");
    let request = update_request(r#"{"attributes":{}}"#);
    let update = update_call(
        target("vs_a", "file_a", &params, None, None),
        &request,
        None,
    );
    let delete = delete_call(target("vs_a", "file_a", &params, None, None));
    let call = match operation {
        Operation::Update => &update,
        Operation::Delete => &delete,
    };
    let mut fixture = Fixture::new("vector_store_file_write");
    let result = run(&mut fixture, &deps, ClientKind::Sync, call, &events).await;
    let Err(FileWriteError::Provider(error)) = result else {
        panic!("a reply that does not parse is a provider error");
    };
    assert_eq!(error.status_code, 200);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(
        error.headers.get("retry-after"),
        Some(&json(r#""3""#)),
        "the transform hands get_error_class the reply's own headers"
    );
    let asked = config.errors();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        (asked[0].1, asked[0].2.clone()),
        (200, error.headers.clone())
    );
    assert_eq!(wire.requests().len(), 1);
}

// The OpenAI files config's own error class carries the reply's status, its parse failure text and its raw headers
#[tokio::test]
async fn openai_transform_parse_failure_uses_the_reply_status() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(
        Reply::json(200, "{not json").with_header("retry-after", "3"),
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://mock.test/v1");
    let request = update_request(r#"{"attributes":{}}"#);
    let call = update_call(
        target("vs_a", "file_a", &params, None, None),
        &request,
        None,
    );
    let mut fixture = Fixture::new("vector_store_file_update");
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(FileWriteError::Provider(error)) = result else {
        panic!("a reply the transform cannot parse is the transform's error");
    };
    assert_eq!(error.status_code, 200);
    assert!(!error.status_code_is_synthesized);
    assert!(
        !error.message.is_empty(),
        "the parser's complaint is the message"
    );
    assert_eq!(error.headers.get("retry-after"), Some(&json(r#""3""#)));
    assert_eq!(wire.requests().len(), 1);
}

// _handle_error masks the reply: content-encoding drops out and content-length is the decoded body's own
#[tokio::test]
async fn status_failure_keeps_the_masked_reply() {
    let config = RecordingConfig::new("http://mock.test/v1");
    let events = Events::new();
    let wire = Wire::replying(
        Reply::json(503, "denied")
            .with_header("content-encoding", "gzip")
            .with_header("content-length", "999"),
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://unused.test");
    let request = update_request(r#"{"attributes":{}}"#);
    let call = update_call(
        target("vs_a", "file_a", &params, None, None),
        &request,
        None,
    );
    let mut fixture = Fixture::new("vector_store_file_update");
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(FileWriteError::Provider(error)) = result else {
        panic!("a 503 is a provider error");
    };
    assert_eq!(error.status_code, 503);
    assert_eq!(error.message, "denied");
    assert_eq!(error.headers.get("content-encoding"), None);
    assert_eq!(error.headers.get("content-length"), Some(&json(r#""6""#)));
    assert_eq!(
        config.errors().len(),
        1,
        "_handle_error asks the provider's class"
    );
}

// LiteLLM builds the URL string and hands the headers to httpx inside the send; betterlite rejects either with no request made
#[tokio::test]
async fn prepare_failures_make_no_request() {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new("not a url");
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://unused.test");
    let request = update_request(r#"{"attributes":{}}"#);
    let mut fixture = Fixture::new("vector_store_file_update");
    let call = update_call(
        target("vs_a", "file_a", &params, None, None),
        &request,
        None,
    );
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(FileWriteError::RequestUrl { url, .. }) = result else {
        panic!("an api_base that is not a URL fails the request URL");
    };
    assert_eq!(url, "not a url/file_a");
    assert_eq!(fixture.input(), Some(&json(r#""""#)));
    assert_eq!(events.recorded(), ["log"]);
    assert!(wire.requests().is_empty());

    let settings = openai_settings();
    let openai = openai_config(&settings, "sk-provider");
    let openai_deps = WriteDeps {
        config: &openai,
        source: &source,
        supplied: None,
    };
    let bad = caller_headers(&[("X-Trace", "bad\nvalue")]);
    let bad_call = update_call(
        target("vs_a", "file_a", &params, Some(&bad), None),
        &request,
        None,
    );
    let result = run(
        &mut fixture,
        &openai_deps,
        ClientKind::Sync,
        &bad_call,
        &events,
    )
    .await;
    let Err(FileWriteError::HeaderValue { name, .. }) = result else {
        panic!("a header value HTTP cannot carry fails the header value");
    };
    assert_eq!(name, "X-Trace");
    assert_eq!(events.recorded(), ["log", "log"]);
    assert!(wire.requests().is_empty());
}

// A transport failure with no reply becomes a synthesized 500 holding the failure's own text
#[tokio::test]
async fn closed_transport_is_a_synthesized_500() {
    let config = RecordingConfig::new("http://mock.test/v1");
    let events = Events::new();
    let wire = Wire::failing(TransportError::Closed, &events);
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://unused.test");
    let request = update_request(r#"{"attributes":{}}"#);
    let call = update_call(
        target("vs_a", "file_a", &params, None, None),
        &request,
        None,
    );
    let mut fixture = Fixture::new("vector_store_file_update");
    let result = run(
        &mut fixture,
        &deps,
        ClientKind::Asynchronous,
        &call,
        &events,
    )
    .await;
    let Err(FileWriteError::Provider(error)) = result else {
        panic!("a closed client is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert_eq!(
        error.message,
        "the client is shutting down and takes no new requests"
    );
    assert_eq!(error.headers, HeaderMap::new());
    assert_eq!(wire.requests().len(), 1, "the send was attempted once");
}

// A reply whose body is not UTF-8 is named by its byte count, as httpx's text decoding would fail on it
#[tokio::test]
async fn binary_reply_body_is_named_by_its_bytes() {
    let config = RecordingConfig::new("http://mock.test/v1");
    let events = Events::new();
    let binary = Reply {
        status: 403,
        headers: http::HeaderMap::new(),
        body: vec![0xff, 0xfe],
    };
    let wire = Wire::replying(binary, &events);
    let source = Source::new(wire.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://unused.test");
    let request = update_request(r#"{"attributes":{}}"#);
    let call = update_call(
        target("vs_a", "file_a", &params, None, None),
        &request,
        None,
    );
    let mut fixture = Fixture::new("vector_store_file_update");
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(FileWriteError::Provider(error)) = result else {
        panic!("a 403 is a provider error");
    };
    assert_eq!(error.status_code, 403);
    assert_eq!(error.message, "2 bytes that are not UTF-8");
    assert_eq!(error.headers.get("content-length"), Some(&json(r#""2""#)));
}

async fn deadline(
    mode: ClientKind,
    operation: Operation,
    timeout: Option<Duration>,
) -> BaseLlmException {
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], "{}"),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    let source = ProxiedSource::new(IMPATIENT);
    let config = RecordingConfig::new(&format!(
        "http://{}/v1/vector_stores/vs_a/files",
        serving.address
    ));
    let events = Events::new();
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = params("http://unused.test");
    let request = update_request(r#"{"attributes":{}}"#);
    let update = update_call(
        target("vs_a", "file_a", &params, None, timeout),
        &request,
        None,
    );
    let delete = delete_call(target("vs_a", "file_a", &params, None, timeout));
    let call = match operation {
        Operation::Update => &update,
        Operation::Delete => &delete,
    };
    let mut fixture = Fixture::new("vector_store_file_write");
    let Err(FileWriteError::Provider(error)) = run(&mut fixture, &deps, mode, call, &events).await
    else {
        panic!("a deadline is a provider error");
    };
    error
}

// The update handlers and the sync delete handler turn httpx.TimeoutException into a 408; the async delete lets it reach _handle_error
#[tokio::test]
async fn file_write_timeout_scope() {
    let async_delete = deadline(ClientKind::Asynchronous, Operation::Delete, None).await;
    assert_eq!(async_delete.status_code, 500);
    assert!(async_delete.status_code_is_synthesized);
    assert!(async_delete.message.contains("read timeout"));
    assert!(async_delete.headers.is_empty());

    let sync_update = deadline(ClientKind::Sync, Operation::Update, None).await;
    assert_eq!(sync_update.status_code, 408);
    assert!(!sync_update.status_code_is_synthesized);
    assert_eq!(
        sync_update.message,
        "litellm.Timeout: Connection timed out after None seconds."
    );

    let given = deadline(
        ClientKind::Sync,
        Operation::Update,
        Some(Duration::from_millis(100)),
    )
    .await;
    assert_eq!(
        given.message,
        "litellm.Timeout: Connection timed out after 0.1 seconds."
    );

    let async_update = deadline(ClientKind::Asynchronous, Operation::Update, None).await;
    assert_eq!(async_update.status_code, 408);
    assert!(
        async_update
            .message
            .starts_with("litellm.Timeout: Connection timed out. Timeout passed=0.1, time taken=")
    );

    let sync_delete = deadline(ClientKind::Sync, Operation::Delete, None).await;
    assert_eq!(sync_delete.status_code, 408);
    assert_eq!(
        sync_delete.message,
        "litellm.Timeout: Connection timed out after None seconds."
    );
}
