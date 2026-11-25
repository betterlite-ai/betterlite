#![cfg(test)]

use std::fs;
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_prometheus::collector_factory::collector::MetricValue;
use betterlite_prometheus::collector_registry::CollectorRegistry;
use betterlite_prometheus::collector_registry::metric::sample::Sample;
use betterlite_prometheus::collector_registry::metric::{CollectorKind, PrometheusMetric};
use betterlite_prometheus::multiprocess::files::decode_db_file;
use betterlite_prometheus::multiprocess::writer::MultiProcessWriter;
use rstest::rstest;
use rust_decimal::Decimal;
use tempfile::TempDir;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use super::{BATCH_DURATION_BUCKETS, CleanupMetrics, RunOutcome, duration_value};

const ROWS_DELETED: &str = "litellm_spend_log_cleanup_rows_deleted_total";
const BATCH_DURATION: &str = "litellm_spend_log_cleanup_batch_duration_seconds";
const ROWS_REMAINING: &str = "litellm_spend_log_cleanup_rows_remaining";
const BATCH_FAILURES: &str = "litellm_spend_log_cleanup_batch_failures_total";
const RUNS: &str = "litellm_spend_log_cleanup_runs_total";

fn finite(value: i64) -> MetricValue {
    MetricValue::Finite(Decimal::new(value, 0))
}

fn series_of(registry: &CollectorRegistry, name: &str, label_value: &str) -> Sample {
    let metric = registry.get(name).expect("a registered instrument");
    metric
        .series(&[Some(label_value.to_owned())])
        .expect("the labelled series")
}

fn registered_registry() -> CollectorRegistry {
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    metrics.record_run(RunOutcome::Completed);
    registry
}

#[derive(Clone, Default)]
struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl LogCapture {
    fn contents(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("UTF-8 log lines")
    }
}

impl Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogCapture {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

fn install_logs() -> (LogCapture, DefaultGuard) {
    let logs = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(logs.clone())
        .finish();
    let guard = set_default(subscriber);
    (logs, guard)
}

fn registration_warnings(logs: &str) -> usize {
    logs.lines()
        .filter(|line| line.contains("Could not register spend-log cleanup metrics"))
        .count()
}

#[test]
fn registration_is_lazy_and_registers_every_instrument() {
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    assert!(registry.descriptions().is_empty());
    metrics.record_run(RunOutcome::Completed);
    assert_eq!(registry.descriptions().len(), 5);
}

#[test]
fn every_sample_name_is_registered_like_prometheus_client() {
    let registry = registered_registry();
    assert!(registry.get(ROWS_DELETED).is_some());
    assert!(
        registry
            .get("litellm_spend_log_cleanup_rows_deleted_created")
            .is_some()
    );
    assert!(registry.get(BATCH_DURATION).is_some());
    assert!(
        registry
            .get("litellm_spend_log_cleanup_batch_duration_seconds_bucket")
            .is_some()
    );
    assert!(
        registry
            .get("litellm_spend_log_cleanup_batch_duration_seconds_sum")
            .is_some()
    );
    assert!(
        registry
            .get("litellm_spend_log_cleanup_batch_duration_seconds_count")
            .is_some()
    );
    assert!(registry.get(ROWS_REMAINING).is_some());
    assert!(
        registry
            .get("litellm_spend_log_cleanup_rows_remaining_total")
            .is_none()
    );
    assert!(registry.get(BATCH_FAILURES).is_some());
    assert!(registry.get(RUNS).is_some());
}

#[rstest]
#[case::rows_deleted(
    "litellm_spend_log_cleanup_rows_deleted",
    CollectorKind::Counter,
    "Rows deleted by the spend-log retention cleanup job",
    "table",
    0
)]
#[case::batch_duration(
    "litellm_spend_log_cleanup_batch_duration_seconds",
    CollectorKind::Histogram,
    "Wall-clock duration of one retention cleanup delete batch",
    "table",
    12
)]
#[case::rows_remaining(
    "litellm_spend_log_cleanup_rows_remaining",
    CollectorKind::Gauge,
    "Expired rows still awaiting deletion, counted only up to SPEND_LOG_CLEANUP_REMAINING_COUNT_CAP so the probe itself cannot scan a large table; a value equal to that cap means at least that many remain",
    "table",
    0
)]
#[case::batch_failures(
    "litellm_spend_log_cleanup_batch_failures",
    CollectorKind::Counter,
    "Retention cleanup delete batches that raised",
    "table",
    0
)]
#[case::runs(
    "litellm_spend_log_cleanup_runs",
    CollectorKind::Counter,
    "Retention cleanup runs, labelled by why the run ended",
    "outcome",
    0
)]
fn instruments_match_the_litellm_definitions(
    #[case] name: &str,
    #[case] kind: CollectorKind,
    #[case] help: &str,
    #[case] label: &str,
    #[case] buckets: usize,
) {
    let descriptions = registered_registry().descriptions();
    let description = descriptions
        .iter()
        .find(|description| description.name == name)
        .expect("a registered instrument");
    assert_eq!(description.kind, kind);
    assert_eq!(description.help, help);
    assert_eq!(description.label_names, vec![label.to_owned()]);
    assert_eq!(description.buckets.len(), buckets);
}

#[test]
fn the_duration_histogram_carries_the_litellm_buckets() {
    let registry = registered_registry();
    let metric = registry
        .get(BATCH_DURATION)
        .expect("the duration histogram");
    assert_eq!(metric.description.buckets, BATCH_DURATION_BUCKETS.to_vec());
}

#[test]
fn record_batch_counts_rows_and_observes_the_duration() {
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    metrics.record_batch("LiteLLM_SpendLogs", 2, 0.5);
    metrics.record_batch("LiteLLM_SpendLogs", 3, 0.25);
    let rows = series_of(&registry, ROWS_DELETED, "LiteLLM_SpendLogs");
    assert_eq!(rows.total, finite(5));
    let batch = series_of(&registry, BATCH_DURATION, "LiteLLM_SpendLogs");
    assert_eq!(batch.total, MetricValue::Finite(Decimal::new(75, 2)));
    assert_eq!(batch.count, 2);
    assert_eq!(batch.buckets, vec![0, 0, 0, 1, 2, 2, 2, 2, 2, 2, 2, 2]);
}

#[test]
fn a_failed_batch_is_counted_per_table() {
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    metrics.record_batch_failure("LiteLLM_SpendLogs");
    metrics.record_batch_failure("LiteLLM_SpendLogs");
    metrics.record_batch_failure("Other");
    let logs = series_of(&registry, BATCH_FAILURES, "LiteLLM_SpendLogs");
    assert_eq!(logs.total, finite(2));
    let other = series_of(&registry, BATCH_FAILURES, "Other");
    assert_eq!(other.total, finite(1));
}

#[test]
fn rows_remaining_is_set_not_incremented() {
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    metrics.set_rows_remaining("LiteLLM_SpendLogs", 5);
    metrics.set_rows_remaining("LiteLLM_SpendLogs", 3);
    let sample = series_of(&registry, ROWS_REMAINING, "LiteLLM_SpendLogs");
    assert_eq!(sample.total, finite(3));
}

#[test]
fn run_outcome_labels_are_the_litellm_literals() {
    let labels: Vec<&str> = [
        RunOutcome::Completed,
        RunOutcome::BudgetExhausted,
        RunOutcome::BatchCapReached,
        RunOutcome::SkippedLocked,
        RunOutcome::SkippedDisabled,
        RunOutcome::Aborted,
    ]
    .into_iter()
    .map(RunOutcome::as_str)
    .collect();
    assert_eq!(
        labels,
        [
            "completed",
            "budget_exhausted",
            "batch_cap_reached",
            "skipped_locked",
            "skipped_disabled",
            "aborted",
        ]
    );
}

#[rstest]
#[case::completed(RunOutcome::Completed, "completed")]
#[case::budget_exhausted(RunOutcome::BudgetExhausted, "budget_exhausted")]
#[case::batch_cap_reached(RunOutcome::BatchCapReached, "batch_cap_reached")]
#[case::skipped_locked(RunOutcome::SkippedLocked, "skipped_locked")]
#[case::skipped_disabled(RunOutcome::SkippedDisabled, "skipped_disabled")]
#[case::aborted(RunOutcome::Aborted, "aborted")]
fn record_run_counts_each_outcome_under_its_label(
    #[case] outcome: RunOutcome,
    #[case] label: &str,
) {
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    metrics.record_run(outcome);
    metrics.record_run(outcome);
    let sample = series_of(&registry, RUNS, label);
    assert_eq!(sample.total, finite(2));
}

#[rstest]
#[case::finite(0.5_f64, MetricValue::Finite(Decimal::new(5, 1)))]
#[case::nan(f64::NAN, MetricValue::NotANumber)]
#[case::positive_infinity(f64::INFINITY, MetricValue::PositiveInfinity)]
#[case::negative_infinity(f64::NEG_INFINITY, MetricValue::NegativeInfinity)]
fn durations_map_to_metric_values(#[case] seconds: f64, #[case] expected: MetricValue) {
    assert_eq!(duration_value(seconds), expected);
}

#[rstest]
#[case::nan(f64::NAN, MetricValue::NotANumber, vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])]
#[case::positive_infinity(f64::INFINITY, MetricValue::PositiveInfinity, vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1])]
#[case::negative_infinity(f64::NEG_INFINITY, MetricValue::NegativeInfinity, vec![1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1])]
fn unusual_durations_keep_their_metric_value(
    #[case] seconds: f64,
    #[case] total: MetricValue,
    #[case] buckets: Vec<u64>,
) {
    let registry = CollectorRegistry::default();
    let metrics = CleanupMetrics::new(&registry);
    metrics.record_batch("LiteLLM_SpendLogs", 0, seconds);
    let sample = series_of(&registry, BATCH_DURATION, "LiteLLM_SpendLogs");
    assert_eq!(sample.total, total);
    assert_eq!(sample.buckets, buckets);
}

#[test]
fn a_duplicate_name_keeps_the_instruments_registered_before_it() {
    let registry = CollectorRegistry::default();
    registry
        .register(PrometheusMetric::new(
            CollectorKind::Gauge,
            ROWS_REMAINING,
            "an instrument that clashes",
            &["table"],
            Vec::new(),
        ))
        .expect("the clashing gauge");
    let (logs, _guard) = install_logs();
    let metrics = CleanupMetrics::new(&registry);
    metrics.record_run(RunOutcome::Completed);
    metrics.record_run(RunOutcome::Completed);
    assert_eq!(registration_warnings(&logs.contents()), 1);
    assert!(registry.get(ROWS_DELETED).is_some());
    assert!(registry.get(BATCH_DURATION).is_some());
    assert!(registry.get(BATCH_FAILURES).is_none());
    assert!(registry.get(RUNS).is_none());
}

#[test]
fn rows_remaining_writes_livemax() {
    let directory = TempDir::new().unwrap();
    let pid = std::process::id();
    let writer = MultiProcessWriter::new(directory.path().to_path_buf(), pid).unwrap();
    let registry = CollectorRegistry::multiprocess(writer);
    let metrics = CleanupMetrics::new(&registry);
    metrics.set_rows_remaining("LiteLLM_SpendLogs", 42);
    let path = directory.path().join(format!("gauge_livemax_{pid}.db"));
    let entries = decode_db_file(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].value.to_bits(), 42.0_f64.to_bits());
    assert!(
        !directory
            .path()
            .join(format!("gauge_all_{pid}.db"))
            .exists()
    );
}
