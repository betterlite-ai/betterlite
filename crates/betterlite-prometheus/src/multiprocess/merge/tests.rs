#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;
use tempfile::TempDir;

use crate::multiprocess::MultiprocessError;
use crate::multiprocess::files::{DbEntry, encode_db_file};
use crate::multiprocess::merge::merge;
use crate::scrape_endpoint::{MetricKind, MetricSample};

fn key(metric: &str, sample: &str, labels: &[(&str, &str)], help: &str) -> String {
    let pairs: BTreeMap<&str, &str> = labels.iter().copied().collect();
    serde_json::to_string(&(metric, sample, pairs, help)).unwrap()
}

fn timed(
    metric: &str,
    sample: &str,
    labels: &[(&str, &str)],
    value: f64,
    timestamp: f64,
) -> DbEntry {
    DbEntry {
        key: key(metric, sample, labels, metric),
        value,
        timestamp,
    }
}

fn entry(metric: &str, sample: &str, labels: &[(&str, &str)], value: f64) -> DbEntry {
    timed(metric, sample, labels, value, 0.0)
}

fn documented(metric: &str, sample: &str, help: &str, value: f64) -> DbEntry {
    DbEntry {
        key: key(metric, sample, &[], help),
        value,
        timestamp: 0.0,
    }
}

fn write_file(dir: &Path, name: &str, entries: &[DbEntry]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, encode_db_file(entries)).unwrap();
    path
}

fn write_histogram(dir: &Path, pid: u32, bound: &str, counts: [f64; 2], sum: f64) -> PathBuf {
    let [one, infinite] = counts;
    write_file(
        dir,
        &format!("histogram_{pid}.db"),
        &[
            entry(
                "request_latency",
                "request_latency_bucket",
                &[("le", bound)],
                one,
            ),
            entry(
                "request_latency",
                "request_latency_bucket",
                &[("le", "+Inf")],
                infinite,
            ),
            entry("request_latency", "request_latency_sum", &[], sum),
        ],
    )
}

fn sample(name: &str, labels: &[(&str, &str)], value: f64) -> MetricSample {
    let pairs = labels
        .iter()
        .map(|(label, label_value)| ((*label).to_owned(), (*label_value).to_owned()))
        .collect();
    MetricSample {
        name: name.to_owned(),
        labels: pairs,
        value,
    }
}

#[test]
fn sums_counters_across_two_pids() {
    let dir = TempDir::new().unwrap();
    let first = write_file(
        dir.path(),
        "counter_1001.db",
        &[entry("requests", "requests_total", &[], 3.0)],
    );
    let second = write_file(
        dir.path(),
        "counter_1002.db",
        &[entry("requests", "requests_total", &[], 4.0)],
    );
    let families = merge(&[first, second]).unwrap();
    assert_eq!(families.len(), 1);
    let family = families.first().unwrap();
    assert_eq!(family.name, "requests");
    assert_eq!(family.kind, MetricKind::Counter);
    assert_eq!(family.samples, vec![sample("requests_total", &[], 7.0)]);
}

#[test]
fn sums_summary_samples() {
    let dir = TempDir::new().unwrap();
    let first = write_file(
        dir.path(),
        "summary_1001.db",
        &[
            entry("queue", "queue_count", &[], 2.0),
            entry("queue", "queue_sum", &[], 9.0),
        ],
    );
    let second = write_file(
        dir.path(),
        "summary_1002.db",
        &[
            entry("queue", "queue_count", &[], 1.0),
            entry("queue", "queue_sum", &[], 4.0),
        ],
    );
    let families = merge(&[first, second]).unwrap();
    let family = families.first().unwrap();
    assert_eq!(family.kind, MetricKind::Summary);
    assert_eq!(
        family.samples,
        vec![
            sample("queue_count", &[], 3.0),
            sample("queue_sum", &[], 13.0),
        ]
    );
}

#[rstest]
#[case("all")]
#[case("liveall")]
fn keeps_each_pid_for_the_all_modes(#[case] mode: &str) {
    let dir = TempDir::new().unwrap();
    let first = write_file(
        dir.path(),
        &format!("gauge_{mode}_1001.db"),
        &[entry("queue_length", "queue_length", &[], 3.0)],
    );
    let second = write_file(
        dir.path(),
        &format!("gauge_{mode}_1002.db"),
        &[entry("queue_length", "queue_length", &[], 5.0)],
    );
    let families = merge(&[first, second]).unwrap();
    let family = families.first().unwrap();
    assert_eq!(family.kind, MetricKind::Gauge);
    assert_eq!(
        family.samples,
        vec![
            sample("queue_length", &[("pid", "1001")], 3.0),
            sample("queue_length", &[("pid", "1002")], 5.0),
        ]
    );
}

#[test]
fn keeps_each_pid_for_an_unknown_mode() {
    let dir = TempDir::new().unwrap();
    let first = write_file(
        dir.path(),
        "gauge_sideways_1001.db",
        &[entry("queue_length", "queue_length", &[], 3.0)],
    );
    let second = write_file(
        dir.path(),
        "gauge_sideways_1002.db",
        &[entry("queue_length", "queue_length", &[], 5.0)],
    );
    let families = merge(&[first, second]).unwrap();
    assert_eq!(
        families.first().unwrap().samples,
        vec![
            sample("queue_length", &[("pid", "1001")], 3.0),
            sample("queue_length", &[("pid", "1002")], 5.0),
        ]
    );
}

#[rstest]
#[case("min", 3.0_f64)]
#[case("livemin", 3.0_f64)]
#[case("max", 5.0_f64)]
#[case("livemax", 5.0_f64)]
#[case("sum", 8.0_f64)]
#[case("livesum", 8.0_f64)]
fn combines_a_gauge_without_its_pid(#[case] mode: &str, #[case] value: f64) {
    let dir = TempDir::new().unwrap();
    let first = write_file(
        dir.path(),
        &format!("gauge_{mode}_1001.db"),
        &[entry("queue_length", "queue_length", &[], 3.0)],
    );
    let second = write_file(
        dir.path(),
        &format!("gauge_{mode}_1002.db"),
        &[entry("queue_length", "queue_length", &[], 5.0)],
    );
    let families = merge(&[first, second]).unwrap();
    let family = families.first().unwrap();
    assert_eq!(family.samples, vec![sample("queue_length", &[], value)]);
}

#[test]
fn combines_a_gauge_without_dropping_its_other_labels() {
    let dir = TempDir::new().unwrap();
    let first = write_file(
        dir.path(),
        "gauge_min_1001.db",
        &[entry(
            "queue_length",
            "queue_length",
            &[("model", "a")],
            5.0,
        )],
    );
    let second = write_file(
        dir.path(),
        "gauge_min_1002.db",
        &[entry(
            "queue_length",
            "queue_length",
            &[("model", "a")],
            3.0,
        )],
    );
    let families = merge(&[first, second]).unwrap();
    assert_eq!(
        families.first().unwrap().samples,
        vec![sample("queue_length", &[("model", "a")], 3.0)]
    );
}

#[rstest]
#[case("mostrecent")]
#[case("livemostrecent")]
fn keeps_the_value_with_the_greatest_timestamp(#[case] mode: &str) {
    let dir = TempDir::new().unwrap();
    let name = format!("gauge_{mode}_1001.db");
    let entries = vec![
        timed("queue_length", "queue_length", &[], 1.0, 5.0),
        timed("queue_length", "queue_length", &[], 2.0, 7.0),
        timed("queue_length", "queue_length", &[], 3.0, 7.0),
        timed("queue_length", "queue_length", &[], 4.0, 6.0),
    ];
    let path = write_file(dir.path(), &name, &entries);
    let families = merge(&[path]).unwrap();
    assert_eq!(
        families.first().unwrap().samples,
        vec![sample("queue_length", &[], 2.0)]
    );
}

#[test]
fn takes_the_mode_of_the_last_gauge_file() {
    let dir = TempDir::new().unwrap();
    let first = write_file(
        dir.path(),
        "gauge_min_1001.db",
        &[entry("queue_length", "queue_length", &[], 5.0)],
    );
    let second = write_file(
        dir.path(),
        "gauge_max_1002.db",
        &[entry("queue_length", "queue_length", &[], 3.0)],
    );
    let families = merge(&[first, second]).unwrap();
    assert_eq!(
        families.first().unwrap().samples,
        vec![sample("queue_length", &[], 5.0)]
    );
}

#[test]
fn min_keeps_the_first_of_two_equal_values() {
    let dir = TempDir::new().unwrap();
    let first = write_file(
        dir.path(),
        "gauge_min_1001.db",
        &[entry("queue_length", "queue_length", &[], 0.0)],
    );
    let second = write_file(
        dir.path(),
        "gauge_min_1002.db",
        &[entry("queue_length", "queue_length", &[], -0.0)],
    );
    let families = merge(&[first, second]).unwrap();
    let samples = &families.first().unwrap().samples;
    assert_eq!(samples.len(), 1);
    assert!(samples.first().unwrap().value.is_sign_positive());
}

#[test]
fn max_keeps_the_first_of_two_equal_values() {
    let dir = TempDir::new().unwrap();
    let first = write_file(
        dir.path(),
        "gauge_max_1001.db",
        &[entry("queue_length", "queue_length", &[], 0.0)],
    );
    let second = write_file(
        dir.path(),
        "gauge_max_1002.db",
        &[entry("queue_length", "queue_length", &[], -0.0)],
    );
    let families = merge(&[first, second]).unwrap();
    let samples = &families.first().unwrap().samples;
    assert_eq!(samples.len(), 1);
    assert!(samples.first().unwrap().value.is_sign_positive());
}

#[test]
fn drops_a_mostrecent_series_whose_timestamps_are_zero() {
    let dir = TempDir::new().unwrap();
    let path = write_file(
        dir.path(),
        "gauge_mostrecent_1001.db",
        &[entry("queue_length", "queue_length", &[], 3.0)],
    );
    let families = merge(&[path]).unwrap();
    assert!(families.first().unwrap().samples.is_empty());
}

#[test]
fn accumulates_histogram_buckets_with_their_count_and_sum() {
    let dir = TempDir::new().unwrap();
    let first = write_histogram(dir.path(), 1001, "1", [2.0_f64, 3.0_f64], 1.0);
    let second = write_histogram(dir.path(), 1002, "1.0", [1.0_f64, 2.0_f64], 0.5);
    let families = merge(&[first, second]).unwrap();
    let family = families.first().unwrap();
    assert_eq!(family.kind, MetricKind::Histogram);
    assert_eq!(
        family.samples,
        vec![
            sample("request_latency_sum", &[], 1.5),
            sample("request_latency_bucket", &[("le", "1.0")], 3.0),
            sample("request_latency_bucket", &[("le", "+Inf")], 8.0),
            sample("request_latency_count", &[], 8.0),
        ]
    );
}

#[test]
fn accumulates_buckets_whose_bounds_compare_equal() {
    let dir = TempDir::new().unwrap();
    let first = write_file(
        dir.path(),
        "histogram_1001.db",
        &[
            entry(
                "request_latency",
                "request_latency_bucket",
                &[("le", "0.0")],
                2.0,
            ),
            entry("request_latency", "request_latency_sum", &[], 1.0),
        ],
    );
    let second = write_file(
        dir.path(),
        "histogram_1002.db",
        &[
            entry(
                "request_latency",
                "request_latency_bucket",
                &[("le", "-0.0")],
                1.0,
            ),
            entry("request_latency", "request_latency_sum", &[], 0.5),
        ],
    );
    let families = merge(&[first, second]).unwrap();
    assert_eq!(
        families.first().unwrap().samples,
        vec![
            sample("request_latency_sum", &[], 1.5),
            sample("request_latency_bucket", &[("le", "0.0")], 3.0),
            sample("request_latency_count", &[], 3.0),
        ]
    );
}

#[test]
fn skips_a_missing_live_gauge_file() {
    let dir = TempDir::new().unwrap();
    let live = write_file(
        dir.path(),
        "counter_1001.db",
        &[entry("requests", "requests_total", &[], 3.0)],
    );
    let missing = dir.path().join("gauge_livesum_1002.db");
    let families = merge(&[missing, live]).unwrap();
    assert_eq!(families.len(), 1);
    assert_eq!(
        families.first().unwrap().samples,
        vec![sample("requests_total", &[], 3.0)]
    );
}

#[test]
fn fails_on_a_missing_counter_file() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("counter_1002.db");
    let error = merge(&[missing]).unwrap_err();
    assert!(matches!(error, MultiprocessError::Read { .. }));
}

#[test]
fn fails_on_a_missing_counter_file_whose_second_part_starts_with_live() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("counter_livesum_1002.db");
    let error = merge(&[missing]).unwrap_err();
    assert!(matches!(error, MultiprocessError::Read { .. }));
}

#[test]
fn fails_on_a_file_name_that_does_not_name_a_gauge_pid() {
    let dir = TempDir::new().unwrap();
    let path = write_file(
        dir.path(),
        "gauge_1001.db",
        &[entry("queue_length", "queue_length", &[], 1.0)],
    );
    let error = merge(&[path]).unwrap_err();
    assert!(matches!(error, MultiprocessError::InvalidFileName { .. }));
}

#[test]
fn fails_on_a_key_that_is_not_a_metric_key() {
    let dir = TempDir::new().unwrap();
    let path = write_file(
        dir.path(),
        "counter_1001.db",
        &[DbEntry {
            key: "not a key".to_owned(),
            value: 1.0,
            timestamp: 0.0,
        }],
    );
    let error = merge(&[path]).unwrap_err();
    assert!(matches!(error, MultiprocessError::InvalidKey { .. }));
}

#[test]
fn fails_on_a_bucket_bound_that_is_not_a_float() {
    let dir = TempDir::new().unwrap();
    let path = write_file(
        dir.path(),
        "histogram_1001.db",
        &[entry(
            "request_latency",
            "request_latency_bucket",
            &[("le", "abc")],
            1.0,
        )],
    );
    let error = merge(&[path]).unwrap_err();
    assert!(matches!(
        &error,
        MultiprocessError::InvalidBucketBound { bound } if bound == "abc"
    ));
    assert_eq!(error.to_string(), "the bucket bound abc is not a float");
}

#[test]
fn fails_on_a_missing_gauge_file_whose_mode_is_not_live() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("gauge_all_1002.db");
    let error = merge(&[missing]).unwrap_err();
    assert!(matches!(error, MultiprocessError::Read { .. }));
}

#[test]
fn fails_on_an_unknown_metric_type() {
    let dir = TempDir::new().unwrap();
    let path = write_file(
        dir.path(),
        "info_1001.db",
        &[entry("target_info", "target_info", &[], 1.0)],
    );
    let error = merge(&[path]).unwrap_err();
    assert!(matches!(
        &error,
        MultiprocessError::InvalidType { kind } if kind == "info"
    ));
    assert_eq!(error.to_string(), "Invalid metric type: info");
}

#[test]
fn fails_on_a_metric_name_that_is_not_an_identifier() {
    let dir = TempDir::new().unwrap();
    let path = write_file(
        dir.path(),
        "counter_1001.db",
        &[entry("1bad", "1bad_total", &[], 1.0)],
    );
    let error = merge(&[path]).unwrap_err();
    assert!(matches!(
        &error,
        MultiprocessError::InvalidName { name } if name == "1bad"
    ));
    assert_eq!(error.to_string(), "Invalid metric name: 1bad");
}

#[test]
fn keeps_the_family_and_sample_order_of_the_files() {
    let dir = TempDir::new().unwrap();
    let first = write_file(
        dir.path(),
        "counter_1001.db",
        &[
            entry("zeta", "zeta_total", &[], 1.0),
            entry("alpha", "alpha_total", &[], 2.0),
        ],
    );
    let second = write_file(
        dir.path(),
        "counter_1002.db",
        &[
            entry("zeta", "zeta_total", &[], 3.0),
            entry("middle", "middle_total", &[], 4.0),
            entry("zeta", "zeta_extra", &[], 5.0),
        ],
    );
    let families = merge(&[first, second]).unwrap();
    let names: Vec<&str> = families.iter().map(|family| family.name.as_str()).collect();
    assert_eq!(names, vec!["zeta", "alpha", "middle"]);
    let zeta = families.first().unwrap();
    let samples: Vec<&str> = zeta
        .samples
        .iter()
        .map(|family_sample| family_sample.name.as_str())
        .collect();
    assert_eq!(samples, vec!["zeta_total", "zeta_extra"]);
}

#[test]
fn takes_the_gauge_pid_from_the_file_name() {
    let dir = TempDir::new().unwrap();
    let path = write_file(
        dir.path(),
        "gauge_all_1003.db",
        &[entry(
            "queue_length",
            "queue_length",
            &[("pid", "stale"), ("model", "a")],
            3.0,
        )],
    );
    let families = merge(&[path]).unwrap();
    assert_eq!(
        families.first().unwrap().samples,
        vec![sample(
            "queue_length",
            &[("model", "a"), ("pid", "1003")],
            3.0
        )]
    );
}

#[test]
fn keeps_the_first_files_help() {
    let dir = TempDir::new().unwrap();
    let first = write_file(
        dir.path(),
        "counter_1001.db",
        &[documented("requests", "requests_total", "first help", 1.0)],
    );
    let second = write_file(
        dir.path(),
        "counter_1002.db",
        &[documented("requests", "requests_total", "second help", 2.0)],
    );
    let families = merge(&[first, second]).unwrap();
    assert_eq!(families.first().unwrap().help, "first help");
}
