#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::azure_storage_logging::logger::CALLBACK_NAME;
use betterlite_logging::callback_logger::CallbackLogger;

use super::support::{Harness, event, settings, settings_with_account_key};
use crate::api::file_storage_contracts::storage::FileStorageBackend;

const SUPPLIED: &str = "https://acct.blob.core.windows.net/files/a.txt";

// the logger uploads to the same filesystem under the same key, so the backend hands it its settings
#[test]
fn the_logger_gets_the_backend_settings() {
    let harness = Harness::new(settings_with_account_key(), &[]);
    let logger = &harness.backend.logger.settings;
    assert_eq!(logger.account_name, "acct");
    assert_eq!(logger.file_system, "files");
    assert_eq!(logger.dfs_endpoint, "https://acct.dfs.core.windows.net");
    assert_eq!(logger.blob_endpoint, "https://acct.blob.core.windows.net");
    assert!(logger.account_key.is_some());
}

#[tokio::test]
async fn logger_overrides_do_nothing() {
    let harness = Harness::new(settings(), &[]);
    let last_flush = harness.backend.logger.last_flush_time();
    harness.clock.advance(Duration::from_secs(3_600));
    let event = event();
    harness
        .backend
        .async_log_success_event(Arc::clone(&event))
        .await
        .expect("a success event is not ours to record");
    harness
        .backend
        .async_log_failure_event(event)
        .await
        .expect("a failure event is not ours to record");
    harness
        .backend
        .delete_file(SUPPLIED)
        .await
        .expect("a delete is a no-op");
    assert_eq!(harness.backend.callback_name(), CALLBACK_NAME);
    assert!(harness.calls().is_empty());
    assert!(harness.backend.logger.queue.pending().is_empty());
    assert_eq!(harness.backend.logger.last_flush_time(), last_flush);
}
