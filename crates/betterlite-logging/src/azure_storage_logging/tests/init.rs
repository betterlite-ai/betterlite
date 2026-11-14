#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::support::{
    ENTRA, FixedIds, ManualClock, Reply, START_SECONDS, ScriptedTransport, environment, fixture,
    payload,
};
use crate::azure_storage_logging::chain::UnwiredChainFactory;
use crate::azure_storage_logging::logger::{AzureBlobStorageLogger, AzureStorageDeps};
use crate::azure_storage_logging::settings::{
    AzureStorageSettings, DEFAULT_ENDPOINT_SUFFIX, SettingsError,
};

#[test]
fn the_logger_initializes_from_the_required_environment() {
    let fixture = fixture(&ENTRA, &[]);
    assert_eq!(fixture.logger.settings.account_name, "acct");
    assert_eq!(fixture.logger.settings.file_system, "logs");
    assert_eq!(fixture.logger.deps.ids.next_id(), "generated-id");
    assert_eq!(fixture.logger.last_flush_time().as_secs(), 1_700_000_000);
}

// CustomBatchLogger.periodic_flush sleeps the interval first, flushes, and stops when the server shuts down
#[tokio::test(start_paused = true)]
async fn the_flush_timer_sends_the_queue_until_shutdown() {
    let mut variables = ENTRA.to_vec();
    variables.push(("DEFAULT_FLUSH_INTERVAL_SECONDS", "1"));
    let lookup = environment(&variables);
    let settings = AzureStorageSettings::from_environment(&lookup).expect("valid settings");
    let transport = ScriptedTransport::new(&[Reply::TOKEN]);
    let tracker = TaskTracker::new();
    let shutdown = CancellationToken::new();
    let logger = AzureBlobStorageLogger::new(
        settings,
        AzureStorageDeps {
            transport: Arc::<ScriptedTransport>::clone(&transport),
            clock: ManualClock::new(START_SECONDS),
            environment: Arc::new(lookup),
            ids: Arc::new(FixedIds),
            chain_factory: Arc::new(UnwiredChainFactory {
                environment: Arc::new(environment(&[])),
            }),
            shutdown: shutdown.clone(),
            tasks: Some(tracker.clone()),
        },
    );
    let _queued = logger.queue.enqueue(payload("id-1"));
    sleep(Duration::from_millis(500)).await;
    assert!(transport.calls().is_empty());
    sleep(Duration::from_secs(1)).await;
    assert!(logger.queue.pending().is_empty());
    assert_eq!(transport.calls().len(), 4);
    shutdown.cancel();
    tracker.close();
    tracker.wait().await;
}

#[test]
fn the_defaults_match_litellms_batch_and_suffix_defaults() {
    let fixture = fixture(&ENTRA, &[]);
    assert_eq!(fixture.settings.endpoint_suffix, DEFAULT_ENDPOINT_SUFFIX);
    assert_eq!(fixture.settings.batch.batch_size, 512);
    assert_eq!(
        fixture.settings.batch.flush_interval,
        Duration::from_secs(5)
    );
    assert_eq!(fixture.settings.batch.max_queue_size, 50_000);
}

#[test]
fn the_batch_environment_variables_override_the_defaults() {
    let variables = [
        ("AZURE_STORAGE_ACCOUNT_NAME", "acct"),
        ("AZURE_STORAGE_FILE_SYSTEM", "logs"),
        ("DEFAULT_BATCH_SIZE", "7"),
        ("DEFAULT_FLUSH_INTERVAL_SECONDS", "11"),
    ];
    let fixture = fixture(&variables, &[]);
    assert_eq!(fixture.settings.batch.batch_size, 7);
    assert_eq!(
        fixture.settings.batch.flush_interval,
        Duration::from_secs(11)
    );
}

#[test]
fn a_missing_account_name_raises_the_required_variable_error() {
    let environment = environment(&[("AZURE_STORAGE_FILE_SYSTEM", "logs")]);
    let error = AzureStorageSettings::from_environment(&environment).expect_err("name is required");
    assert!(matches!(error, SettingsError::MissingAccountName));
    assert_eq!(
        error.to_string(),
        "Missing required environment variable: AZURE_STORAGE_ACCOUNT_NAME"
    );
}

#[test]
fn a_missing_file_system_raises_the_required_variable_error() {
    let environment = environment(&[("AZURE_STORAGE_ACCOUNT_NAME", "acct")]);
    let error =
        AzureStorageSettings::from_environment(&environment).expect_err("system is required");
    assert!(matches!(error, SettingsError::MissingFileSystem));
    assert_eq!(
        error.to_string(),
        "Missing required environment variable: AZURE_STORAGE_FILE_SYSTEM"
    );
}
