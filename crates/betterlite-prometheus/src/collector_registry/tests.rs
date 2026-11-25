#![cfg(test)]

use std::fs;
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

use rstest::rstest;
use rust_decimal::Decimal;
use tempfile::TempDir;

use crate::collector_factory::collector::{Collector as _, MetricValue};
use crate::collector_registry::metric::binding;
use crate::collector_registry::metric::sample::added;
use crate::collector_registry::metric::{
    Bucket, BucketError, CollectorKind, PrometheusMetric, SeriesRemoval, prepare_buckets,
};
use crate::collector_registry::{CollectorRegistry, CollectorRegistryError};
use crate::multiprocess::files::{DbEntry, decode_db_file};
use crate::multiprocess::writer::MultiProcessWriter;

fn finite(value: i64) -> MetricValue {
    MetricValue::Finite(Decimal::new(value, 0))
}

fn multiprocess_registry(directory: &TempDir, pid: u32) -> CollectorRegistry {
    let writer = MultiProcessWriter::new(directory.path().to_path_buf(), pid)
        .expect("a writer over the folder");
    CollectorRegistry::multiprocess(writer)
}

fn written_entries(directory: &TempDir, name: &str) -> Vec<DbEntry> {
    let bytes = fs::read(directory.path().join(name)).expect("the metric file");
    decode_db_file(&bytes).expect("a readable metric file")
}

fn registry_with(collector: PrometheusMetric) -> CollectorRegistry {
    let registry = CollectorRegistry::default();
    registry.register(collector).unwrap();
    registry
}

#[test]
fn a_counter_registers_its_stripped_name_total_and_created() {
    let registry = registry_with(PrometheusMetric::new(
        CollectorKind::Counter,
        "litellm_guardrail_errors_total",
        "errors",
        &[],
        Vec::new(),
    ));
    assert!(registry.get("litellm_guardrail_errors").is_some());
    assert!(registry.get("litellm_guardrail_errors_total").is_some());
    assert!(registry.get("litellm_guardrail_errors_created").is_some());
    assert_eq!(registry.descriptions()[0].name, "litellm_guardrail_errors");
}

#[test]
fn a_counter_without_the_suffix_keeps_its_name() {
    let registry = registry_with(PrometheusMetric::new(
        CollectorKind::Counter,
        "requests",
        "requests",
        &[],
        Vec::new(),
    ));
    assert!(registry.get("requests").is_some());
    assert!(registry.get("requests_total").is_some());
    assert!(registry.get("requests_created").is_some());
}

#[test]
fn a_gauge_registers_only_its_own_name() {
    let registry = registry_with(PrometheusMetric::new(
        CollectorKind::Gauge,
        "in_flight",
        "in flight",
        &[],
        Vec::new(),
    ));
    assert!(registry.get("in_flight").is_some());
    assert!(registry.get("in_flight_total").is_none());
    assert!(registry.get("in_flight_created").is_none());
}

#[test]
fn a_histogram_registers_bucket_sum_count_and_created() {
    let bounds = prepare_buckets(&[Bucket::Seconds(0.5)]).unwrap();
    let registry = registry_with(PrometheusMetric::new(
        CollectorKind::Histogram,
        "latency",
        "latency",
        &[],
        bounds,
    ));
    assert!(registry.get("latency").is_some());
    assert!(registry.get("latency_bucket").is_some());
    assert!(registry.get("latency_sum").is_some());
    assert!(registry.get("latency_count").is_some());
    assert!(registry.get("latency_created").is_some());
    assert!(registry.get("latency_total").is_none());
}

#[test]
fn a_duplicate_collector_names_every_clashing_sample() {
    let registry = registry_with(PrometheusMetric::new(
        CollectorKind::Counter,
        "requests",
        "requests",
        &[],
        Vec::new(),
    ));
    let error = registry
        .register(PrometheusMetric::new(
            CollectorKind::Counter,
            "requests",
            "requests",
            &[],
            Vec::new(),
        ))
        .unwrap_err();
    assert_eq!(
        error,
        CollectorRegistryError::DuplicatedTimeseries {
            names: vec![
                "requests".to_owned(),
                "requests_total".to_owned(),
                "requests_created".to_owned(),
            ],
        }
    );
}

#[test]
fn a_gauge_clashing_with_a_counter_base_name_is_refused() {
    let registry = registry_with(PrometheusMetric::new(
        CollectorKind::Counter,
        "requests",
        "requests",
        &[],
        Vec::new(),
    ));
    let error = registry
        .register(PrometheusMetric::new(
            CollectorKind::Gauge,
            "requests",
            "requests",
            &[],
            Vec::new(),
        ))
        .unwrap_err();
    assert_eq!(
        error,
        CollectorRegistryError::DuplicatedTimeseries {
            names: vec!["requests".to_owned()],
        }
    );
}

#[test]
fn a_refused_duplicate_leaves_the_registry_as_it_was() {
    let registry = registry_with(PrometheusMetric::new(
        CollectorKind::Counter,
        "requests",
        "requests",
        &[],
        Vec::new(),
    ));
    let _error = registry
        .register(PrometheusMetric::new(
            CollectorKind::Counter,
            "requests",
            "requests",
            &[],
            Vec::new(),
        ))
        .unwrap_err();
    assert_eq!(registry.descriptions().len(), 1);
    assert_eq!(registry.metrics().len(), 1);
}

#[rstest]
#[case::base("requests")]
#[case::total("requests_total")]
#[case::created("requests_created")]
fn the_lookup_returns_the_collector_for_every_one_of_its_names(#[case] name: &str) {
    let registry = registry_with(PrometheusMetric::new(
        CollectorKind::Counter,
        "requests",
        "requests",
        &["model"],
        Vec::new(),
    ));
    let collector = registry.get(name).unwrap();
    assert_eq!(collector.description.name, "requests");
    assert_eq!(collector.description.label_names, vec!["model".to_owned()]);
}

#[test]
fn descriptions_list_kind_help_labelnames_and_buckets_in_registration_order() {
    let bounds = prepare_buckets(&[Bucket::Seconds(0.5), Bucket::Seconds(2.0)]).unwrap();
    let registry = CollectorRegistry::default();
    registry
        .register(PrometheusMetric::new(
            CollectorKind::Counter,
            "first",
            "first help",
            &["model"],
            Vec::new(),
        ))
        .unwrap();
    registry
        .register(PrometheusMetric::new(
            CollectorKind::Histogram,
            "second",
            "second help",
            &["team", "user"],
            bounds,
        ))
        .unwrap();
    let descriptions = registry.descriptions();
    assert_eq!(descriptions.len(), 2);
    assert_eq!(descriptions[0].kind, CollectorKind::Counter);
    assert_eq!(descriptions[0].help, "first help");
    assert_eq!(descriptions[0].label_names, vec!["model".to_owned()]);
    assert!(descriptions[0].buckets.is_empty());
    assert_eq!(descriptions[1].kind, CollectorKind::Histogram);
    assert_eq!(descriptions[1].help, "second help");
    assert_eq!(
        descriptions[1].label_names,
        vec!["team".to_owned(), "user".to_owned()]
    );
    assert_eq!(
        descriptions[1].buckets,
        vec![Bucket::Seconds(0.5), Bucket::Seconds(2.0), Bucket::Infinite]
    );
}

#[test]
fn prepare_buckets_appends_the_infinite_bound() {
    let prepared = prepare_buckets(&[Bucket::Seconds(0.5), Bucket::Seconds(2.0)]).unwrap();
    assert_eq!(
        prepared,
        vec![Bucket::Seconds(0.5), Bucket::Seconds(2.0), Bucket::Infinite]
    );
}

#[test]
fn prepare_buckets_keeps_an_existing_infinite_bound() {
    let prepared = prepare_buckets(&[Bucket::Seconds(0.5), Bucket::Infinite]).unwrap();
    assert_eq!(prepared, vec![Bucket::Seconds(0.5), Bucket::Infinite]);
}

#[test]
fn prepare_buckets_keeps_equal_bounds() {
    let prepared = prepare_buckets(&[Bucket::Seconds(1.0), Bucket::Seconds(1.0)]).unwrap();
    assert_eq!(
        prepared,
        vec![Bucket::Seconds(1.0), Bucket::Seconds(1.0), Bucket::Infinite]
    );
}

#[test]
fn prepare_buckets_treats_a_positive_infinity_as_the_infinite_bound() {
    let prepared =
        prepare_buckets(&[Bucket::Seconds(0.5), Bucket::Seconds(f64::INFINITY)]).unwrap();
    assert_eq!(prepared, vec![Bucket::Seconds(0.5), Bucket::Infinite]);
}

#[test]
fn prepare_buckets_rejects_an_unsorted_list() {
    let error = prepare_buckets(&[Bucket::Seconds(2.0), Bucket::Seconds(0.5)]).unwrap_err();
    assert_eq!(error, BucketError::NotSorted);
    assert_eq!(error.to_string(), "Buckets not in sorted order");
}

#[test]
fn prepare_buckets_rejects_not_a_number() {
    let error = prepare_buckets(&[Bucket::Seconds(0.5), Bucket::Seconds(f64::NAN)]).unwrap_err();
    assert_eq!(error, BucketError::NotSorted);
}

#[test]
fn prepare_buckets_rejects_an_empty_list() {
    let error = prepare_buckets(&[]).unwrap_err();
    assert_eq!(error, BucketError::TooFew);
    assert_eq!(error.to_string(), "Must have at least two buckets");
}

#[test]
fn prepare_buckets_rejects_a_single_infinite_bound() {
    assert_eq!(
        prepare_buckets(&[Bucket::Infinite]).unwrap_err(),
        BucketError::TooFew
    );
}

#[test]
fn a_counter_inc_adds_to_its_total() {
    let metric = PrometheusMetric::new(
        CollectorKind::Counter,
        "requests",
        "requests",
        &[],
        Vec::new(),
    );
    metric.labels(&[]).inc(finite(2));
    metric.labels(&[]).inc(finite(3));
    let sample = metric.series(&[]).unwrap();
    assert_eq!(sample.total, finite(5));
}

#[test]
fn a_counter_set_replaces_its_total() {
    let metric = PrometheusMetric::new(
        CollectorKind::Counter,
        "requests",
        "requests",
        &[],
        Vec::new(),
    );
    metric.labels(&[]).inc(finite(2));
    metric.labels(&[]).set(finite(9));
    assert_eq!(metric.series(&[]).unwrap().total, finite(9));
}

#[test]
fn labels_keep_each_series_apart() {
    let metric = PrometheusMetric::new(
        CollectorKind::Gauge,
        "in_flight",
        "in flight",
        &["model"],
        Vec::new(),
    );
    metric.labels(&[("model", "a")]).set(finite(2));
    metric.labels(&[("model", "b")]).set(finite(3));
    assert_eq!(
        metric.series(&[Some("a".to_owned())]).unwrap().total,
        finite(2)
    );
    assert_eq!(
        metric.series(&[Some("b".to_owned())]).unwrap().total,
        finite(3)
    );
}

#[test]
fn a_missing_label_value_lands_under_none() {
    let metric = PrometheusMetric::new(
        CollectorKind::Gauge,
        "in_flight",
        "in flight",
        &["model", "team"],
        Vec::new(),
    );
    metric.labels(&[("model", "a")]).set(finite(4));
    let sample = metric
        .series(&[Some("a".to_owned()), None])
        .expect("the series is there under None");
    assert_eq!(sample.total, finite(4));
}

#[test]
fn a_histogram_observation_counts_every_bucket_it_falls_into() {
    let bounds = prepare_buckets(&[Bucket::Seconds(0.5), Bucket::Seconds(2.0)]).unwrap();
    let metric = PrometheusMetric::new(CollectorKind::Histogram, "latency", "latency", &[], bounds);
    metric
        .labels(&[])
        .observe(MetricValue::Finite(Decimal::new(1, 0)));
    metric
        .labels(&[])
        .observe(MetricValue::Finite(Decimal::new(25, 2)));
    let sample = metric.series(&[]).unwrap();
    assert_eq!(sample.count, 2);
    assert_eq!(sample.total, MetricValue::Finite(Decimal::new(125, 2)));
    assert_eq!(sample.buckets, vec![1, 2, 2]);
}

#[test]
fn an_infinite_observation_counts_only_the_infinite_bucket() {
    let bounds = prepare_buckets(&[Bucket::Seconds(0.5)]).unwrap();
    let metric = PrometheusMetric::new(CollectorKind::Histogram, "latency", "latency", &[], bounds);
    metric.labels(&[]).observe(MetricValue::PositiveInfinity);
    assert_eq!(metric.series(&[]).unwrap().buckets, vec![0, 1]);
}

#[test]
fn a_negative_infinite_observation_counts_every_bucket() {
    let bounds = prepare_buckets(&[Bucket::Seconds(0.5), Bucket::Seconds(2.0)]).unwrap();
    let metric = PrometheusMetric::new(CollectorKind::Histogram, "latency", "latency", &[], bounds);
    metric.labels(&[]).observe(MetricValue::NegativeInfinity);
    assert_eq!(metric.series(&[]).unwrap().buckets, vec![1, 1, 1]);
}

#[test]
fn not_a_number_reaches_no_bucket() {
    let bounds = prepare_buckets(&[Bucket::Seconds(0.5)]).unwrap();
    let metric = PrometheusMetric::new(CollectorKind::Histogram, "latency", "latency", &[], bounds);
    metric.labels(&[]).observe(MetricValue::NotANumber);
    let sample = metric.series(&[]).unwrap();
    assert_eq!(sample.buckets, vec![0, 0]);
    assert_eq!(sample.total, MetricValue::NotANumber);
}

#[test]
fn remove_drops_the_series_it_was_called_on() {
    let metric = PrometheusMetric::new(
        CollectorKind::Gauge,
        "in_flight",
        "in flight",
        &["model"],
        Vec::new(),
    );
    metric.labels(&[("model", "a")]).set(finite(4));
    assert_eq!(
        metric.remove_series(&[Some("a".to_owned())]),
        SeriesRemoval::Removed
    );
    assert!(metric.series(&[Some("a".to_owned())]).is_none());
}

#[test]
fn removing_an_absent_series_is_not_an_error() {
    let metric = PrometheusMetric::new(
        CollectorKind::Gauge,
        "in_flight",
        "in flight",
        &["model"],
        Vec::new(),
    );
    assert_eq!(
        metric.remove_series(&[Some("missing".to_owned())]),
        SeriesRemoval::Absent
    );
}

#[test]
fn removing_without_label_dimensions_is_unsupported() {
    let metric = PrometheusMetric::new(
        CollectorKind::Gauge,
        "in_flight",
        "in flight",
        &[],
        Vec::new(),
    );
    assert_eq!(metric.remove_series(&[]), SeriesRemoval::Unsupported);
}

#[test]
fn removing_with_the_wrong_label_count_is_unsupported() {
    let metric = PrometheusMetric::new(
        CollectorKind::Gauge,
        "in_flight",
        "in flight",
        &["model"],
        Vec::new(),
    );
    assert_eq!(metric.remove_series(&[]), SeriesRemoval::Unsupported);
    assert_eq!(
        metric.remove_series(&[Some("a".to_owned()), Some("b".to_owned())]),
        SeriesRemoval::Unsupported
    );
}

#[test]
fn remove_through_the_collector_trait_drops_the_current_series() {
    let metric = PrometheusMetric::new(
        CollectorKind::Gauge,
        "in_flight",
        "in flight",
        &["model"],
        Vec::new(),
    );
    let labelled = metric.labels(&[("model", "a")]);
    labelled.set(finite(4));
    labelled.remove();
    assert!(metric.series(&[Some("a".to_owned())]).is_none());
}

#[test]
fn a_total_past_the_decimal_range_becomes_an_infinity() {
    let positive = PrometheusMetric::new(
        CollectorKind::Counter,
        "requests",
        "requests",
        &[],
        Vec::new(),
    );
    positive.labels(&[]).set(MetricValue::Finite(Decimal::MAX));
    positive.labels(&[]).inc(finite(1));
    assert_eq!(
        positive.series(&[]).unwrap().total,
        MetricValue::PositiveInfinity
    );
    let negative = PrometheusMetric::new(
        CollectorKind::Counter,
        "requests",
        "requests",
        &[],
        Vec::new(),
    );
    negative.labels(&[]).set(MetricValue::Finite(Decimal::MIN));
    negative.labels(&[]).inc(finite(-1));
    assert_eq!(
        negative.series(&[]).unwrap().total,
        MetricValue::NegativeInfinity
    );
}

#[test]
fn kinds_report_the_prometheus_type_names() {
    assert_eq!(CollectorKind::Counter.as_str(), "counter");
    assert_eq!(CollectorKind::Gauge.as_str(), "gauge");
    assert_eq!(CollectorKind::Histogram.as_str(), "histogram");
}

#[test]
fn adding_infinities_follows_the_ieee_rules() {
    assert_eq!(
        added(MetricValue::PositiveInfinity, MetricValue::NegativeInfinity),
        MetricValue::NotANumber
    );
    assert_eq!(
        added(finite(1), MetricValue::PositiveInfinity),
        MetricValue::PositiveInfinity
    );
    assert_eq!(
        added(MetricValue::NotANumber, finite(1)),
        MetricValue::NotANumber
    );
}

#[test]
fn multiprocess_a_handle_kept_before_register_writes() {
    let directory = TempDir::new().unwrap();
    let registry = multiprocess_registry(&directory, 31);
    let metric = PrometheusMetric::new(
        CollectorKind::Counter,
        "litellm_requests",
        "Requests served",
        &["model"],
        Vec::new(),
    );
    let handle = metric.labels(&[("model", "gpt-4o")]);
    registry.register(metric).unwrap();
    handle.inc(finite(2));
    let entries = written_entries(&directory, "counter_31.db");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].value.to_bits(), 2.0_f64.to_bits());
}

#[test]
fn multiprocess_a_labels_handle_writes_its_series() {
    let directory = TempDir::new().unwrap();
    let registry = multiprocess_registry(&directory, 32);
    registry
        .register(PrometheusMetric::new(
            CollectorKind::Gauge,
            "litellm_in_flight",
            "In flight",
            &["model"],
            Vec::new(),
        ))
        .unwrap();
    let metric = registry.get("litellm_in_flight").unwrap();
    metric.labels(&[("model", "a")]).inc(finite(1));
    metric.labels(&[("model", "b")]).inc(finite(3));
    let entries = written_entries(&directory, "gauge_all_32.db");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].value.to_bits(), 1.0_f64.to_bits());
    assert_eq!(entries[1].value.to_bits(), 3.0_f64.to_bits());
}

#[test]
fn multiprocess_an_unlabelled_metric_writes_zero_entries_at_register() {
    let directory = TempDir::new().unwrap();
    let registry = multiprocess_registry(&directory, 33);
    let bounds = prepare_buckets(&[Bucket::Seconds(0.5)]).unwrap();
    let metric = PrometheusMetric::new(
        CollectorKind::Histogram,
        "litellm_latency",
        "Latency",
        &[],
        bounds,
    );
    assert!(!directory.path().join("histogram_33.db").exists());
    registry.register(metric).unwrap();
    let entries = written_entries(&directory, "histogram_33.db");
    assert_eq!(entries.len(), 3);
    assert!(
        entries
            .iter()
            .all(|entry| entry.value.to_bits() == 0.0_f64.to_bits())
    );
}

#[test]
fn multiprocess_a_default_registry_writes_nothing() {
    let directory = TempDir::new().unwrap();
    let _writer = MultiProcessWriter::new(directory.path().to_path_buf(), 34).unwrap();
    let registry = CollectorRegistry::default();
    let metric = PrometheusMetric::new(
        CollectorKind::Counter,
        "litellm_requests",
        "Requests served",
        &[],
        Vec::new(),
    );
    registry.register(metric.clone()).unwrap();
    metric.inc(finite(1));
    assert!(registry.multiprocess_dir().is_none());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn multiprocess_a_second_writer_is_refused() {
    let bound_folder = TempDir::new().unwrap();
    let refused_folder = TempDir::new().unwrap();
    let bound_registry = multiprocess_registry(&bound_folder, 35);
    let metric = PrometheusMetric::new(
        CollectorKind::Counter,
        "litellm_requests",
        "Requests served",
        &[],
        Vec::new(),
    );
    bound_registry.register(metric.clone()).unwrap();
    let refused_registry = multiprocess_registry(&refused_folder, 35);
    let error = refused_registry.register(metric).unwrap_err();
    assert_eq!(
        error,
        CollectorRegistryError::MultiprocessBound {
            name: "litellm_requests".to_owned(),
            path: bound_folder.path().to_path_buf(),
        }
    );
    assert_eq!(
        error.to_string(),
        format!(
            "the metric litellm_requests is already bound to the multiprocess writer in {}",
            bound_folder.path().display()
        )
    );
    assert_eq!(fs::read_dir(refused_folder.path()).unwrap().count(), 0);
}

#[test]
fn multiprocess_binding_the_same_writer_again_is_fine() {
    let directory = TempDir::new().unwrap();
    let writer = Arc::new(MultiProcessWriter::new(directory.path().to_path_buf(), 36).unwrap());
    let metric = PrometheusMetric::new(
        CollectorKind::Counter,
        "litellm_requests",
        "Requests served",
        &[],
        Vec::new(),
    );
    binding::bind_writer(&metric, &writer).unwrap();
    binding::bind_writer(&metric, &writer).unwrap();
    assert!(metric.multiprocess.lock().unwrap().is_some());
}

#[test]
fn multiprocess_a_removed_series_keeps_its_entry_and_continues() {
    let directory = TempDir::new().unwrap();
    let registry = multiprocess_registry(&directory, 37);
    let metric = PrometheusMetric::new(
        CollectorKind::Counter,
        "litellm_requests",
        "Requests served",
        &["model"],
        Vec::new(),
    );
    registry.register(metric.clone()).unwrap();
    let handle = metric.labels(&[("model", "a")]);
    handle.inc(finite(2));
    handle.remove();
    assert!(metric.series(&[Some("a".to_owned())]).is_none());
    let kept = written_entries(&directory, "counter_37.db");
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].value.to_bits(), 2.0_f64.to_bits());
    metric.labels(&[("model", "a")]).inc(finite(3));
    let continued = written_entries(&directory, "counter_37.db");
    assert_eq!(continued.len(), 1);
    assert_eq!(continued[0].value.to_bits(), 5.0_f64.to_bits());
}

#[test]
fn multiprocess_dir_is_the_writers_folder() {
    let directory = TempDir::new().unwrap();
    let registry = multiprocess_registry(&directory, 38);
    assert_eq!(registry.multiprocess_dir(), Some(directory.path()));
}

#[test]
fn concurrent_registrations_of_one_name_leave_one_collector() {
    const REGISTRATIONS: usize = 16;
    let directory = TempDir::new().unwrap();
    let registry = multiprocess_registry(&directory, 39);
    let barrier = Barrier::new(REGISTRATIONS);
    let outcomes = Mutex::new(Vec::with_capacity(REGISTRATIONS));
    thread::scope(|scope| {
        let spawned = (0..REGISTRATIONS)
            .map(|_slot| {
                scope.spawn(|| {
                    let metric = PrometheusMetric::new(
                        CollectorKind::Counter,
                        "litellm_shared",
                        "Shared",
                        &[],
                        Vec::new(),
                    );
                    let _started = barrier.wait();
                    outcomes.lock().unwrap().push(registry.register(metric));
                })
            })
            .count();
        assert_eq!(spawned, REGISTRATIONS);
    });
    let results = outcomes.into_inner().unwrap();
    let registered = results.iter().filter(|outcome| outcome.is_ok()).count();
    let refused = results.iter().filter(|outcome| outcome.is_err()).count();
    assert_eq!(registered, 1);
    assert_eq!(refused, REGISTRATIONS.saturating_sub(1));
    assert_eq!(registry.metrics().len(), 1);
    assert_eq!(written_entries(&directory, "counter_39.db").len(), 1);
}
