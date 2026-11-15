#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::gcs_client::gcs_storage_auth::GcsBucketBase;
use crate::gcs_client::gcs_storage_auth::initialization_behaviors::initialize;
use crate::gcs_logger::gcs_bucket_logging::GcsDeps;

use super::super::tests::support::{
    Answer, FakeAuth, RecordingClient, SequenceIds, SetClock, now, open,
};
use super::{
    BASE_BATCH_SIZE, BASE_FLUSH_INTERVAL_SECONDS, BATCH_SIZE_ENV, DEFAULT_BATCH_SIZE,
    DEFAULT_FLUSH_INTERVAL_SECONDS, DEFAULT_QUEUE_SIZE, DEFAULT_USE_BATCHED_LOGGING,
    FLUSH_INTERVAL_ENV, GcsSettings, QUEUE_SIZE_ENV, USE_BATCHED_LOGGING_ENV, start,
};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn defaults_apply_without_environment_settings() {
    let settings = GcsSettings::from_environment(&environment(&[])).expect("the defaults parse");
    assert_eq!(settings.batch_size, DEFAULT_BATCH_SIZE);
    assert_eq!(settings.batch_size, 2048);
    assert_eq!(
        settings.flush_interval.as_secs(),
        DEFAULT_FLUSH_INTERVAL_SECONDS
    );
    assert_eq!(settings.flush_interval.as_secs(), 20);
    assert_eq!(settings.use_batched_logging, DEFAULT_USE_BATCHED_LOGGING);
    assert!(settings.use_batched_logging);
    assert_eq!(settings.queue_capacity, DEFAULT_QUEUE_SIZE);
    assert_eq!(settings.queue_capacity, 1000);
}

#[test]
fn environment_values_override_every_setting() {
    let settings = GcsSettings::from_environment(&environment(&[
        (BATCH_SIZE_ENV, "5"),
        (FLUSH_INTERVAL_ENV, "7"),
        (USE_BATCHED_LOGGING_ENV, "false"),
        (QUEUE_SIZE_ENV, "3"),
    ]))
    .expect("the settings parse");
    assert_eq!(settings.batch_size, 5);
    assert_eq!(settings.flush_interval.as_secs(), 7);
    assert!(!settings.use_batched_logging);
    assert_eq!(settings.queue_capacity, 3);
}

#[test]
fn a_zero_batch_size_and_flush_interval_take_the_base_logger_defaults() {
    let settings = GcsSettings::from_environment(&environment(&[
        (BATCH_SIZE_ENV, "0"),
        (FLUSH_INTERVAL_ENV, "0"),
    ]))
    .expect("the zero settings parse");
    assert_eq!(settings.batch_size, BASE_BATCH_SIZE);
    assert_eq!(settings.batch_size, 512);
    assert_eq!(
        settings.flush_interval.as_secs(),
        BASE_FLUSH_INTERVAL_SECONDS
    );
    assert_eq!(settings.flush_interval.as_secs(), 5);
}

#[test]
fn a_batch_size_and_flush_interval_of_one_are_kept() {
    let settings = GcsSettings::from_environment(&environment(&[
        (BATCH_SIZE_ENV, "1"),
        (FLUSH_INTERVAL_ENV, "1"),
    ]))
    .expect("the settings parse");
    assert_eq!(settings.batch_size, 1);
    assert_eq!(settings.flush_interval.as_secs(), 1);
}

#[test]
fn an_empty_setting_counts_as_unset() {
    let settings = GcsSettings::from_environment(&environment(&[
        (BATCH_SIZE_ENV, ""),
        (FLUSH_INTERVAL_ENV, ""),
        (USE_BATCHED_LOGGING_ENV, ""),
        (QUEUE_SIZE_ENV, ""),
    ]))
    .expect("the empty settings fall back to the defaults");
    assert_eq!(settings.batch_size, DEFAULT_BATCH_SIZE);
    assert_eq!(
        settings.flush_interval.as_secs(),
        DEFAULT_FLUSH_INTERVAL_SECONDS
    );
    assert_eq!(settings.use_batched_logging, DEFAULT_USE_BATCHED_LOGGING);
    assert_eq!(settings.queue_capacity, DEFAULT_QUEUE_SIZE);
}

#[test]
fn a_non_numeric_batch_size_is_refused() {
    let error = GcsSettings::from_environment(&environment(&[(BATCH_SIZE_ENV, "many")]))
        .expect_err("the value is not a number");
    assert!(error.to_string().contains(BATCH_SIZE_ENV));
}

#[test]
fn a_negative_flush_interval_is_refused() {
    let error = GcsSettings::from_environment(&environment(&[(FLUSH_INTERVAL_ENV, "-1")]))
        .expect_err("the interval cannot be negative");
    assert!(error.to_string().contains(FLUSH_INTERVAL_ENV));
}

#[test]
fn a_non_boolean_batched_logging_setting_is_refused() {
    let error =
        GcsSettings::from_environment(&environment(&[(USE_BATCHED_LOGGING_ENV, "sometimes")]))
            .expect_err("the value is not a bool");
    assert!(error.to_string().contains(USE_BATCHED_LOGGING_ENV));
}

#[tokio::test]
async fn the_logger_starts_empty_at_the_clock_time() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    assert_eq!(harness.queue_len(), 0);
    assert_eq!(harness.last_flush_time(), now());
}

#[tokio::test]
async fn start_refuses_a_setting_it_cannot_read() {
    let base: GcsBucketBase<FakeAuth, RecordingClient> = initialize(
        Arc::new(environment(&[(BATCH_SIZE_ENV, "many")])),
        Some("bucket".to_owned()),
        FakeAuth,
        RecordingClient::new(Answer::ok("{}")),
    )
    .expect("the GCS base initializes");
    let result = start(
        base,
        GcsDeps {
            clock: Arc::new(SetClock::new(now())),
            ids: Arc::new(SequenceIds::default()),
            tasks: None,
            shutdown: CancellationToken::new(),
        },
    );
    let Err(error) = result else {
        panic!("the invalid setting is refused");
    };
    assert!(error.to_string().contains(BATCH_SIZE_ENV));
}
