#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{Result as IoResult, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use rstest::rstest;
use rust_decimal::Decimal;
use tempfile::TempDir;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::collector_factory::collector::{Collector as _, MetricUpdate, MetricValue};
use crate::collector_registry::metric::binding;
use crate::collector_registry::metric::gauge_mode::GaugeMode;
use crate::collector_registry::metric::{Bucket, CollectorKind, PrometheusMetric, prepare_buckets};
use crate::multiprocess::collector::MultiProcessCollector;
use crate::multiprocess::files::{DbEntry, decode_db_file, decode_used, encode_db_file};
use crate::multiprocess::writer::{MultiProcessWriter, SeriesWrite};
use crate::scrape_endpoint::{MetricFamily, MetricKind, MetricSample};

fn writer(directory: &TempDir, pid: u32) -> MultiProcessWriter {
    MultiProcessWriter::new(directory.path().to_path_buf(), pid).unwrap()
}

fn finite(value: i64) -> MetricValue {
    MetricValue::Finite(Decimal::new(value, 0))
}

fn file_entries(directory: &TempDir, name: &str) -> Vec<DbEntry> {
    let bytes = fs::read(directory.path().join(name)).unwrap();
    decode_db_file(&bytes).unwrap()
}

fn sample_of(entry: &DbEntry) -> (String, BTreeMap<String, String>) {
    let (_metric, sample, labels, _help) =
        serde_json::from_str::<(String, String, BTreeMap<String, String>, String)>(&entry.key)
            .unwrap();
    (sample, labels)
}

fn bucket_sample(bound: &str) -> (String, BTreeMap<String, String>) {
    let labels = BTreeMap::from([("le".to_owned(), bound.to_owned())]);
    ("litellm_latency_bucket".to_owned(), labels)
}

// 800 labelled series is past one 64 KiB page, so the file has to grow before the last of them fits
fn write_workers(writer: &MultiProcessWriter, metric: &PrometheusMetric) {
    for index in 0..800_u32 {
        writer.update(
            &metric.description,
            &[format!("worker-{index}")],
            MetricUpdate::Inc(finite(1)),
        );
    }
}

#[test]
fn new_refuses_an_empty_path_and_a_file() {
    let message = "env PROMETHEUS_MULTIPROC_DIR is not set or not a directory";
    let empty = MultiProcessWriter::new(PathBuf::new(), 1).unwrap_err();
    assert_eq!(empty.to_string(), message);
    let file = tempfile::NamedTempFile::new().unwrap();
    let not_a_folder = MultiProcessWriter::new(file.path().to_path_buf(), 1).unwrap_err();
    assert_eq!(not_a_folder.to_string(), message);
}

#[test]
fn a_first_write_leaves_the_bytes_encode_db_file_builds() {
    let directory = TempDir::new().unwrap();
    let writer = writer(&directory, 7);
    let metric = PrometheusMetric::new(
        CollectorKind::Counter,
        "litellm_requests",
        "Requests served",
        &["model"],
        Vec::new(),
    );
    writer.update(
        &metric.description,
        &["gpt-4o".to_owned()],
        MetricUpdate::Inc(finite(2)),
    );
    let key =
        r#"["litellm_requests","litellm_requests_total",{"model":"gpt-4o"},"Requests served"]"#;
    let expected = encode_db_file(&[DbEntry {
        key: key.to_owned(),
        value: 2.0,
        timestamp: 0.0,
    }]);
    let written = fs::read(directory.path().join("counter_7.db")).unwrap();
    assert_eq!(written, expected);
    assert_eq!(written.len(), 0x0001_0000);
}

#[rstest]
#[case::all(GaugeMode::All, "gauge_all_9.db")]
#[case::livesum(GaugeMode::LiveSum, "gauge_livesum_9.db")]
#[case::livemax(GaugeMode::LiveMax, "gauge_livemax_9.db")]
fn the_three_gauge_modes_write_three_files(#[case] mode: GaugeMode, #[case] file: &str) {
    let directory = TempDir::new().unwrap();
    let writer = writer(&directory, 9);
    let metric = PrometheusMetric::new(
        CollectorKind::Gauge,
        "litellm_in_flight",
        "In flight",
        &[],
        Vec::new(),
    )
    .with_multiprocess_mode(mode);
    writer.update(&metric.description, &[], MetricUpdate::Inc(finite(1)));
    assert_eq!(
        file_entries(&directory, file)[0].value.to_bits(),
        1.0_f64.to_bits()
    );
}

#[test]
fn a_gauge_set_replaces_the_stored_value() {
    let directory = TempDir::new().unwrap();
    let writer = writer(&directory, 10);
    let metric = PrometheusMetric::new(CollectorKind::Gauge, "rows", "Rows", &[], Vec::new());
    writer.update(&metric.description, &[], MetricUpdate::Inc(finite(2)));
    writer.update(&metric.description, &[], MetricUpdate::Set(finite(10)));
    assert_eq!(
        file_entries(&directory, "gauge_all_10.db")[0]
            .value
            .to_bits(),
        10.0_f64.to_bits()
    );
}

#[test]
fn a_file_grows_to_128_kib_once_its_entries_pass_64_kib() {
    let directory = TempDir::new().unwrap();
    let writer = writer(&directory, 11);
    let metric = PrometheusMetric::new(
        CollectorKind::Counter,
        "litellm_fanout",
        "Fanout",
        &["worker"],
        Vec::new(),
    );
    write_workers(&writer, &metric);
    let bytes = fs::read(directory.path().join("counter_11.db")).unwrap();
    assert_eq!(bytes.len(), 0x0002_0000);
    assert!(decode_used(&bytes).unwrap() > 0x0001_0000);
    assert_eq!(decode_db_file(&bytes).unwrap().len(), 800);
}

#[test]
fn an_entry_that_fills_the_page_exactly_does_not_grow_it() {
    let directory = TempDir::new().unwrap();
    let mut file =
        super::file::FileState::open(directory.path(), super::FileKind::Counter, 41).unwrap();
    file.ensure(&"k".repeat(65_500)).unwrap();
    let bytes = fs::read(directory.path().join("counter_41.db")).unwrap();
    assert_eq!(bytes.len(), 0x0001_0000);
    assert_eq!(decode_used(&bytes).unwrap(), 0x0001_0000);
}

#[test]
fn a_histogram_writes_its_sum_and_one_count_per_bucket() {
    let directory = TempDir::new().unwrap();
    let writer = writer(&directory, 13);
    let bounds = prepare_buckets(&[Bucket::Seconds(0.5), Bucket::Seconds(1.0)]).unwrap();
    let metric = PrometheusMetric::new(
        CollectorKind::Histogram,
        "litellm_latency",
        "Latency",
        &[],
        bounds,
    );
    let half = MetricValue::Finite(Decimal::new(5, 1));
    writer.update(&metric.description, &[], MetricUpdate::Observe(half));
    writer.update(&metric.description, &[], MetricUpdate::Observe(finite(3)));
    writer.update(
        &metric.description,
        &[],
        MetricUpdate::Observe(MetricValue::NotANumber),
    );
    let entries = file_entries(&directory, "histogram_13.db");
    let samples: Vec<(String, BTreeMap<String, String>)> = entries.iter().map(sample_of).collect();
    assert_eq!(
        samples,
        [
            ("litellm_latency_sum".to_owned(), BTreeMap::new()),
            bucket_sample("0.5"),
            bucket_sample("1.0"),
            bucket_sample("+Inf"),
        ]
    );
    assert!(entries[0].value.is_nan());
    assert_eq!(entries[1].value.to_bits(), 1.0_f64.to_bits());
    assert_eq!(entries[2].value.to_bits(), 0.0_f64.to_bits());
    assert_eq!(entries[3].value.to_bits(), 1.0_f64.to_bits());
    assert!(
        entries
            .iter()
            .all(|entry| entry.timestamp.to_bits() == 0.0_f64.to_bits())
    );
}

#[test]
fn a_created_series_writes_every_key_at_zero() {
    let directory = TempDir::new().unwrap();
    let writer = writer(&directory, 15);
    let bounds = prepare_buckets(&[Bucket::Seconds(0.5)]).unwrap();
    let metric = PrometheusMetric::new(
        CollectorKind::Histogram,
        "litellm_latency",
        "Latency",
        &[],
        bounds,
    );
    writer.write_series(&metric.description, &[], SeriesWrite::Create);
    let entries = file_entries(&directory, "histogram_15.db");
    let samples: Vec<String> = entries.iter().map(|entry| sample_of(entry).0).collect();
    assert_eq!(
        samples,
        [
            "litellm_latency_sum",
            "litellm_latency_bucket",
            "litellm_latency_bucket"
        ]
    );
    assert!(
        entries
            .iter()
            .all(|entry| entry.value.to_bits() == 0.0_f64.to_bits())
    );
}

#[test]
fn a_removal_writes_nothing() {
    let directory = TempDir::new().unwrap();
    let writer = writer(&directory, 16);
    let metric = PrometheusMetric::new(
        CollectorKind::Counter,
        "litellm_requests",
        "Requests",
        &[],
        Vec::new(),
    );
    writer.update(&metric.description, &[], MetricUpdate::Remove);
    assert!(!directory.path().join("counter_16.db").exists());
}

#[test]
fn a_removal_of_a_bound_series_logs_prometheus_clients_warning() {
    let directory = TempDir::new().unwrap();
    let metric = PrometheusMetric::new(
        CollectorKind::Gauge,
        "litellm_in_flight",
        "In flight",
        &["model"],
        Vec::new(),
    );
    let writer = Arc::new(writer(&directory, 23));
    binding::bind_writer(&metric, &writer).unwrap();
    let (logs, _guard) = install_logs();
    metric.labels(&[("model", "a")]).remove();
    let logged = logs.contents();
    assert!(logged.contains("WARN"));
    assert!(
        logged.contains("Removal of labels has not been implemented in  multi-process mode yet.")
    );
}

#[test]
fn a_file_an_earlier_writer_left_is_continued_not_rewritten() {
    let directory = TempDir::new().unwrap();
    let key = r#"["litellm_requests","litellm_requests_total",{},"Requests"]"#;
    let before = encode_db_file(&[DbEntry {
        key: key.to_owned(),
        value: 3.0,
        timestamp: 0.0,
    }]);
    fs::write(directory.path().join("counter_17.db"), &before).unwrap();
    let writer = writer(&directory, 17);
    let metric = PrometheusMetric::new(
        CollectorKind::Counter,
        "litellm_requests",
        "Requests",
        &[],
        Vec::new(),
    );
    writer.update(&metric.description, &[], MetricUpdate::Inc(finite(2)));
    let bytes = fs::read(directory.path().join("counter_17.db")).unwrap();
    assert_eq!(bytes.len(), 0x0001_0000);
    assert_eq!(bytes.get(..4), before.get(..4));
    let entries = decode_db_file(&bytes).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].value.to_bits(), 5.0_f64.to_bits());
}

#[test]
fn a_writer_continues_a_file_that_already_grew() {
    let directory = TempDir::new().unwrap();
    let metric = PrometheusMetric::new(
        CollectorKind::Counter,
        "litellm_fanout",
        "Fanout",
        &["worker"],
        Vec::new(),
    );
    let first = writer(&directory, 43);
    write_workers(&first, &metric);
    drop(first);
    let second = writer(&directory, 43);
    second.update(
        &metric.description,
        &["worker-0".to_owned()],
        MetricUpdate::Inc(finite(1)),
    );
    let bytes = fs::read(directory.path().join("counter_43.db")).unwrap();
    assert_eq!(bytes.len(), 0x0002_0000);
    let entries = decode_db_file(&bytes).unwrap();
    assert_eq!(entries.len(), 800);
    let worker = entries
        .iter()
        .find(|entry| {
            sample_of(entry)
                .1
                .get("worker")
                .is_some_and(|value| value == "worker-0")
        })
        .unwrap();
    assert_eq!(worker.value.to_bits(), 2.0_f64.to_bits());
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

#[test]
fn a_write_into_a_folder_removed_after_new_logs_an_error() {
    let directory = TempDir::new().unwrap();
    let writer = writer(&directory, 19);
    let metric = PrometheusMetric::new(
        CollectorKind::Counter,
        "litellm_requests",
        "Requests",
        &[],
        Vec::new(),
    );
    let (logs, _guard) = install_logs();
    fs::remove_dir_all(directory.path()).unwrap();
    writer.update(&metric.description, &[], MetricUpdate::Inc(finite(1)));
    let logged = logs.contents();
    assert!(logged.contains("ERROR"));
    assert!(logged.contains("writing a metric to the multiprocess folder failed"));
    assert!(logged.contains("counter_19.db"));
}

// a counter, a gauge and a histogram in one folder, so the collector merges one family of each kind
fn write_three_families(writer: &MultiProcessWriter) {
    let requests = PrometheusMetric::new(
        CollectorKind::Counter,
        "litellm_requests",
        "Requests served",
        &["model"],
        Vec::new(),
    );
    writer.update(
        &requests.description,
        &["gpt-4o".to_owned()],
        MetricUpdate::Inc(finite(2)),
    );
    let in_flight = PrometheusMetric::new(
        CollectorKind::Gauge,
        "litellm_in_flight",
        "In flight",
        &[],
        Vec::new(),
    )
    .with_multiprocess_mode(GaugeMode::LiveSum);
    writer.update(&in_flight.description, &[], MetricUpdate::Inc(finite(3)));
    let latency = PrometheusMetric::new(
        CollectorKind::Histogram,
        "litellm_latency",
        "Latency",
        &[],
        prepare_buckets(&[Bucket::Seconds(0.5), Bucket::Seconds(1.0)]).unwrap(),
    );
    writer.update(&latency.description, &[], MetricUpdate::Observe(finite(1)));
}

#[test]
fn the_collector_reads_the_files_back_as_families() {
    let directory = TempDir::new().unwrap();
    let writer = writer(&directory, 21);
    write_three_families(&writer);
    let families = MultiProcessCollector::new(directory.path())
        .unwrap()
        .collect()
        .unwrap();
    let by_name: BTreeMap<&str, &MetricFamily> = families
        .iter()
        .map(|family| (family.name.as_str(), family))
        .collect();
    assert_eq!(
        by_name.keys().copied().collect::<Vec<&str>>(),
        ["litellm_in_flight", "litellm_latency", "litellm_requests"]
    );
    let requests = by_name["litellm_requests"];
    assert_eq!(requests.kind, MetricKind::Counter);
    assert_eq!(requests.help, "Requests served");
    assert_eq!(
        requests.samples,
        [MetricSample {
            name: "litellm_requests_total".to_owned(),
            labels: vec![("model".to_owned(), "gpt-4o".to_owned())],
            value: 2.0,
        }]
    );
    let gauge = by_name["litellm_in_flight"];
    assert_eq!(gauge.kind, MetricKind::Gauge);
    assert_eq!(gauge.samples[0].value.to_bits(), 3.0_f64.to_bits());
    let latency = by_name["litellm_latency"];
    assert_eq!(latency.kind, MetricKind::Histogram);
    let sum = latency
        .samples
        .iter()
        .find(|sample| sample.name == "litellm_latency_sum")
        .unwrap();
    assert_eq!(sum.value.to_bits(), 1.0_f64.to_bits());
}
