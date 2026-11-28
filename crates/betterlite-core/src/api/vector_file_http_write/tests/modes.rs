use std::time::Duration;

use http::Method;
use rstest::rstest;

use super::run;
use super::support::{
    Events, Fixture, RecordingConfig, Reply, Source, Wire, delete_call, params, target,
    update_call, update_request,
};
use crate::api::vector_file_http_write::service::WriteDeps;
use crate::api::vector_store_http_write::service::{ClientKind, Operation, SuppliedClient};

const FILE: &str = r#"{"id":"file_a","object":"vector_store.file"}"#;

// A supplied client of the called mode answers, so the pool never runs; the timeout reaches it as given, null included
#[rstest]
#[case::sync_update_some(ClientKind::Sync, Operation::Update, Some(Duration::from_secs(7)))]
#[case::sync_update_null(ClientKind::Sync, Operation::Update, None)]
#[case::async_update_some(
    ClientKind::Asynchronous,
    Operation::Update,
    Some(Duration::from_secs(7))
)]
#[case::async_update_null(ClientKind::Asynchronous, Operation::Update, None)]
#[case::sync_delete_some(ClientKind::Sync, Operation::Delete, Some(Duration::from_secs(7)))]
#[case::sync_delete_null(ClientKind::Sync, Operation::Delete, None)]
#[case::async_delete_some(
    ClientKind::Asynchronous,
    Operation::Delete,
    Some(Duration::from_secs(7))
)]
#[case::async_delete_null(ClientKind::Asynchronous, Operation::Delete, None)]
#[tokio::test]
async fn supplied_client_of_the_called_mode_answers(
    #[case] mode: ClientKind,
    #[case] operation: Operation,
    #[case] timeout: Option<Duration>,
) {
    let events = Events::new();
    let supplied = Wire::replying(Reply::json(200, FILE), &events);
    let pooled = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(pooled.clone());
    let config = RecordingConfig::new("http://mock.test/v1");
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: Some(SuppliedClient {
            kind: mode,
            transport: &supplied,
        }),
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
    run(&mut fixture, &deps, mode, call, &events)
        .await
        .expect("the write succeeds");
    assert!(source.calls().is_empty(), "the pool is never asked");
    let requests = supplied.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout, timeout);
    let expected = match operation {
        Operation::Update => Method::POST,
        Operation::Delete => Method::DELETE,
    };
    assert_eq!(requests[0].method, expected);
    assert!(pooled.requests().is_empty());
}

// The sync handler drops an async client and the async handler a sync one, and either then asks the pool with litellm_params' ssl_verify as configured, null included
#[rstest]
#[case::async_handler_drops_sync(ClientKind::Asynchronous, ClientKind::Sync, None)]
#[case::async_handler_drops_sync_unverified(
    ClientKind::Asynchronous,
    ClientKind::Sync,
    Some(false)
)]
#[case::sync_handler_drops_async(ClientKind::Sync, ClientKind::Asynchronous, None)]
#[case::sync_handler_drops_async_unverified(
    ClientKind::Sync,
    ClientKind::Asynchronous,
    Some(false)
)]
#[tokio::test]
async fn wrong_mode_client_selects_the_pool(
    #[case] mode: ClientKind,
    #[case] supplied: ClientKind,
    #[case] ssl_verify: Option<bool>,
) {
    let events = Events::new();
    let unused = Wire::replying(Reply::json(200, FILE), &events);
    let pooled = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(pooled.clone());
    let config = RecordingConfig::new("http://mock.test/v1");
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: Some(SuppliedClient {
            kind: supplied,
            transport: &unused,
        }),
    };
    let mut configured = params("http://unused.test");
    configured.ssl_verify = ssl_verify;
    let request = update_request(r#"{"attributes":{}}"#);
    let call = update_call(
        target(
            "vs_a",
            "file_a",
            &configured,
            None,
            Some(Duration::from_secs(7)),
        ),
        &request,
        None,
    );
    let mut fixture = Fixture::new("vector_store_file_update");
    run(&mut fixture, &deps, mode, &call, &events)
        .await
        .expect("the update succeeds");
    assert_eq!(source.calls(), [ssl_verify]);
    assert!(unused.requests().is_empty());
    let requests = pooled.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout, Some(Duration::from_secs(7)));
}
