use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex as AsyncMutex;

use crate::zerobus::client::ZerobusIngestClient;
use crate::zerobus::client::types::ZerobusIngestFailure;
use crate::zerobus::logger::ZerobusLogger;
use crate::zerobus::logger::connection::SecretSource;
use crate::zerobus::logger::delivery::RowIngest;

use super::support::{
    Buffer, FakeIngest, as_row_ingest, as_secret_source, connection, connection_secrets, deps,
    expected_ids, logger, params, proxied_client, push, queued_ids, row, subscribe,
};

#[tokio::test]
async fn zerobus_batch_send_sends_the_queued_rows_in_one_insert() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let logger = logger(params(10, 3600, false), client);
    assert_eq!(push(&logger, "a"), 1);
    assert_eq!(push(&logger, "b"), 2);
    logger.inner.flush_queue(false).await;
    assert_eq!(ingest.inserted(), 1);
    assert_eq!(ingest.ids(), expected_ids(&["a", "b"]));
    assert!(queued_ids(&logger).is_empty());
}

#[tokio::test]
async fn zerobus_batch_send_reports_the_client_failure() {
    let client = ZerobusIngestClient {
        connection: connection("main.traces"),
        http: proxied_client(),
        clock: Arc::new(|| 0.0_f64),
        timeout: Duration::from_secs(1),
        token: AsyncMutex::new(None),
    };
    let row_ingest: &dyn RowIngest = &client;
    let failure = row_ingest
        .ingest(&[row("a")])
        .await
        .expect_err("the token form rejects a table that is not fully qualified");
    assert!(!failure.retryable);
    assert!(failure.detail.contains("main.traces"), "{}", failure.detail);
}

#[tokio::test]
async fn zerobus_batch_send_sends_nothing_for_an_empty_queue() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let logger = logger(params(10, 3600, false), client);
    logger.inner.flush_queue(false).await;
    assert_eq!(ingest.inserted(), 0);
}

#[tokio::test]
async fn zerobus_batch_send_keeps_the_rows_on_a_retryable_failure() {
    let ingest = FakeIngest::failing(ZerobusIngestFailure {
        detail: "503".to_owned(),
        retryable: true,
    });
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let logger = logger(params(10, 3600, false), client);
    assert_eq!(push(&logger, "a"), 1);
    logger.inner.flush_queue(false).await;
    assert_eq!(ingest.inserted(), 1);
    assert_eq!(queued_ids(&logger), expected_ids(&["a"]));
}

#[tokio::test]
async fn zerobus_batch_send_drops_the_rows_on_a_permanent_failure() {
    let ingest = FakeIngest::failing(ZerobusIngestFailure {
        detail: "400".to_owned(),
        retryable: false,
    });
    let client: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let logger = logger(params(10, 3600, false), client);
    let buffer = Buffer::default();
    let guard = subscribe(&buffer);
    assert_eq!(push(&logger, "a"), 1);
    logger.inner.flush_queue(false).await;
    drop(guard);
    assert_eq!(ingest.inserted(), 1);
    assert!(queued_ids(&logger).is_empty());
    assert!(
        buffer.contents().contains("cannot be retried"),
        "{}",
        buffer.contents()
    );
}

#[tokio::test]
async fn zerobus_batch_send_keeps_the_rows_when_the_connection_cannot_be_resolved() {
    let secrets = Arc::new(connection_secrets());
    let source: Arc<dyn SecretSource> = as_secret_source(&secrets);
    let mut configured = deps(params(10, 3600, false), None);
    configured.secrets = source;
    let logger = ZerobusLogger::new(configured).expect("the connection resolves");
    secrets.set("ZEROBUS_TABLE_NAME", "");
    let buffer = Buffer::default();
    let guard = subscribe(&buffer);
    assert_eq!(push(&logger, "a"), 1);
    logger.inner.flush_queue(false).await;
    drop(guard);
    assert_eq!(queued_ids(&logger), expected_ids(&["a"]));
    assert!(
        buffer.contents().contains("cannot be resolved"),
        "{}",
        buffer.contents()
    );
}
