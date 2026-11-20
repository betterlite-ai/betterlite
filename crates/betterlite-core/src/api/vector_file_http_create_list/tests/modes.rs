use std::time::Duration;

use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_openai::api::vector_file_contracts::records::FileParams;
use rstest::rstest;

use super::support::{
    CallRequest, Events, Fixture, IMPATIENT, ProxiedSource, RecordingConfig, Reply, Source, Wire,
    file_call, file_deps, file_params, openai_config, openai_settings, params,
};
use super::{FILE, run};
use crate::api::vector_file_http_create_list::error::FileTransportError;
use crate::api::vector_file_http_create_list::service::Operation;
use crate::api::vector_store_http_write::error::BaseLlmException;
use crate::api::vector_store_http_write::service::ClientKind;

#[rstest]
#[case::sync_timed(ClientKind::Sync, Some(Duration::from_secs(7)))]
#[case::sync_plain(ClientKind::Sync, None)]
#[case::async_timed(ClientKind::Asynchronous, Some(Duration::from_secs(7)))]
#[case::async_plain(ClientKind::Asynchronous, None)]
#[tokio::test]
async fn create_passes_timeout_in_both_modes(
    #[case] mode: ClientKind,
    #[case] timeout: Option<Duration>,
) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let files = params("http://mock.test/v1");
    let request = CallRequest {
        timeout,
        ..CallRequest::new("vs_a", &files)
    };
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let mut fixture = Fixture::new("vector_store_file_create");
    let call = file_call(&request, Operation::Create, &original, None);
    run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the create succeeds");
    assert_eq!(wire.requests().len(), 1);
    assert_eq!(wire.requests()[0].timeout, timeout);
}

#[rstest]
#[case::sync_timed(ClientKind::Sync, Some(Duration::from_secs(7)))]
#[case::sync_plain(ClientKind::Sync, None)]
#[case::async_timed(ClientKind::Asynchronous, Some(Duration::from_secs(7)))]
#[case::async_plain(ClientKind::Asynchronous, None)]
#[tokio::test]
async fn list_never_passes_timeout(#[case] mode: ClientKind, #[case] timeout: Option<Duration>) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = file_deps(&config, &source);
    let files = params("http://mock.test/v1");
    let request = CallRequest {
        timeout,
        ..CallRequest::new("vs_a", &files)
    };
    let query = file_params(r#"{"limit":2}"#);
    let mut fixture = Fixture::new("vector_store_file_list");
    let call = file_call(&request, Operation::List, &query, None);
    run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the list succeeds");
    assert_eq!(wire.requests().len(), 1);
    assert!(wire.requests()[0].timeout.is_none());
}

async fn timed_create_error<'call>(
    source: &ProxiedSource,
    request: &CallRequest<'call>,
    original: &'call FileParams,
    mode: ClientKind,
    events: &Events,
) -> BaseLlmException {
    let slow = serve(
        reply("200 OK", &[("content-type", "application/json")], "{}"),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    let config = RecordingConfig::new(&format!(
        "http://{}/v1/vector_stores/vs_a/files",
        slow.address
    ));
    let deps = file_deps(&config, source);
    let mut fixture = Fixture::new("vector_store_file_create");
    let call = file_call(request, Operation::Create, original, None);
    let result = run(&mut fixture, &deps, mode, &call, events).await;
    let Err(FileTransportError::Provider(error)) = result else {
        panic!("a timeout is a provider error");
    };
    error
}

#[tokio::test]
async fn create_timeout_messages_follow_the_mode() {
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let files = params("http://unused.test");
    let source = ProxiedSource::new(IMPATIENT);
    let events = Events::new();
    let request = CallRequest {
        timeout: Some(Duration::from_millis(100)),
        ..CallRequest::new("vs_a", &files)
    };
    let sync_error =
        timed_create_error(&source, &request, &original, ClientKind::Sync, &events).await;
    assert_eq!(sync_error.status_code, 408);
    assert!(!sync_error.status_code_is_synthesized);
    assert_eq!(
        sync_error.message,
        "litellm.Timeout: Connection timed out after 0.1 seconds."
    );
    let async_error = timed_create_error(
        &source,
        &request,
        &original,
        ClientKind::Asynchronous,
        &events,
    )
    .await;
    assert_eq!(async_error.status_code, 408);
    assert!(
        async_error
            .message
            .starts_with("litellm.Timeout: Connection timed out. Timeout passed=0.1, time taken=")
    );
    assert!(async_error.message.ends_with(" seconds"));
}

#[tokio::test]
async fn create_timeout_without_a_value_names_none() {
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let files = params("http://unused.test");
    let source = ProxiedSource::new(IMPATIENT);
    let events = Events::new();
    let request = CallRequest::new("vs_a", &files);
    let sync_error =
        timed_create_error(&source, &request, &original, ClientKind::Sync, &events).await;
    assert_eq!(sync_error.status_code, 408);
    assert_eq!(
        sync_error.message,
        "litellm.Timeout: Connection timed out after None seconds."
    );
    // no per-call timeout on the async side either, so the pooled client's read timeout is the only deadline
    let async_error = timed_create_error(
        &source,
        &request,
        &original,
        ClientKind::Asynchronous,
        &events,
    )
    .await;
    assert_eq!(async_error.status_code, 408);
    assert!(
        async_error
            .message
            .starts_with("litellm.Timeout: Connection timed out. Timeout passed=None, time taken=")
    );
}

#[tokio::test]
async fn list_timeout_is_a_synthesized_500() {
    let query = file_params(r#"{"limit":2}"#);
    let files = params("http://unused.test");
    let slow = serve(
        reply("200 OK", &[("content-type", "application/json")], "{}"),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    let config = RecordingConfig::new(&format!(
        "http://{}/v1/vector_stores/vs_a/files",
        slow.address
    ));
    let events = Events::new();
    let source = ProxiedSource::new(IMPATIENT);
    let deps = file_deps(&config, &source);
    // the timeout argument is seven seconds, so a list that passed it would wait for the slow answer
    let request = CallRequest {
        timeout: Some(Duration::from_secs(7)),
        ..CallRequest::new("vs_a", &files)
    };
    let mut fixture = Fixture::new("vector_store_file_list");
    let call = file_call(&request, Operation::List, &query, None);
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(FileTransportError::Provider(error)) = result else {
        panic!("a list timeout is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert!(error.message.contains("read timeout"));
    assert!(error.headers.is_empty());
}
