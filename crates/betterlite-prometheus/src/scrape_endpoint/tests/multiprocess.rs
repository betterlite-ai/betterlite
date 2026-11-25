use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tempfile::TempDir;
use tower::ServiceExt as _;

use crate::multiprocess::collector::MultiProcessCollector;
use crate::multiprocess::files::{DbEntry, encode_db_file};
use crate::scrape_endpoint::ScrapeSource;
use crate::scrape_endpoint::app::make_metrics_app;

use super::request;

fn key(metric: &str, sample: &str, labels: &[(&str, &str)]) -> String {
    let pairs: BTreeMap<&str, &str> = labels.iter().copied().collect();
    serde_json::to_string(&(metric, sample, pairs, metric)).unwrap()
}

fn entry(metric: &str, sample: &str, labels: &[(&str, &str)], value: f64) -> DbEntry {
    DbEntry {
        key: key(metric, sample, labels),
        value,
        timestamp: 0.0,
    }
}

fn write_file(dir: &Path, name: &str, entries: &[DbEntry]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, encode_db_file(entries)).unwrap();
    path
}

fn source_of(dir: &Path) -> ScrapeSource {
    ScrapeSource::Multiprocess(MultiProcessCollector::new(dir).unwrap())
}

fn body_text(source: &ScrapeSource, accept: &str, names: &[&str]) -> String {
    let scrape = request(accept, "", names);
    String::from_utf8(source.render(&scrape).unwrap()).unwrap()
}

#[test]
fn multiprocess_renders_a_counter_without_created() {
    let dir = TempDir::new().unwrap();
    let _first = write_file(
        dir.path(),
        "counter_1001.db",
        &[entry("requests", "requests_total", &[("model", "a")], 3.0)],
    );
    let _second = write_file(
        dir.path(),
        "counter_1002.db",
        &[entry("requests", "requests_total", &[("model", "a")], 4.0)],
    );
    let source = source_of(dir.path());
    assert_eq!(
        body_text(&source, "", &[]),
        "# HELP requests_total requests\n# TYPE requests_total counter\nrequests_total{model=\"a\"} 7.0\n"
    );
}

#[test]
fn multiprocess_renders_a_gauge_with_its_pid_label() {
    let dir = TempDir::new().unwrap();
    let _file = write_file(
        dir.path(),
        "gauge_all_1001.db",
        &[entry("queue_length", "queue_length", &[], 3.0)],
    );
    let source = source_of(dir.path());
    assert_eq!(
        body_text(&source, "", &[]),
        "# HELP queue_length queue_length\n# TYPE queue_length gauge\nqueue_length{pid=\"1001\"} 3.0\n"
    );
}

#[test]
fn multiprocess_renders_a_summary() {
    let dir = TempDir::new().unwrap();
    let _file = write_file(
        dir.path(),
        "summary_1001.db",
        &[
            entry("queue", "queue_count", &[], 2.0),
            entry("queue", "queue_sum", &[], 9.0),
        ],
    );
    let source = source_of(dir.path());
    assert_eq!(
        body_text(&source, "", &[]),
        "# HELP queue queue\n# TYPE queue summary\nqueue_count 2.0\nqueue_sum 9.0\n"
    );
}

#[test]
fn multiprocess_renders_a_histogram() {
    let dir = TempDir::new().unwrap();
    let _file = write_file(
        dir.path(),
        "histogram_1001.db",
        &[
            entry(
                "request_latency",
                "request_latency_bucket",
                &[("le", "1")],
                2.0,
            ),
            entry(
                "request_latency",
                "request_latency_bucket",
                &[("le", "+Inf")],
                3.0,
            ),
            entry("request_latency", "request_latency_sum", &[], 1.5),
        ],
    );
    let source = source_of(dir.path());
    assert_eq!(
        body_text(&source, "", &[]),
        "# HELP request_latency request_latency\n# TYPE request_latency histogram\nrequest_latency_sum 1.5\nrequest_latency_bucket{le=\"1.0\"} 2.0\nrequest_latency_bucket{le=\"+Inf\"} 5.0\nrequest_latency_count 5.0\n"
    );
}

#[test]
fn multiprocess_renders_openmetrics() {
    let dir = TempDir::new().unwrap();
    let _file = write_file(
        dir.path(),
        "counter_1001.db",
        &[entry("requests", "requests_total", &[("model", "a")], 3.0)],
    );
    let source = source_of(dir.path());
    assert_eq!(
        body_text(&source, "application/openmetrics-text", &[]),
        "# HELP requests requests\n# TYPE requests counter\nrequests_total{model=\"a\"} 3.0\n# EOF\n"
    );
}

#[test]
fn multiprocess_filters_a_name_scrape() {
    let dir = TempDir::new().unwrap();
    let _file = write_file(
        dir.path(),
        "counter_1001.db",
        &[entry("requests", "requests_total", &[], 3.0)],
    );
    let source = source_of(dir.path());
    assert_eq!(
        body_text(&source, "", &["requests_total"]),
        "# HELP requests_total requests\n# TYPE requests_total counter\nrequests_total 3.0\n"
    );
    assert_eq!(body_text(&source, "", &["unknown"]), "");
    assert_eq!(
        body_text(&source, "application/openmetrics-text", &["unknown"]),
        "# EOF\n"
    );
}

#[test]
fn multiprocess_reads_the_folder_at_every_scrape() {
    let dir = TempDir::new().unwrap();
    let source = source_of(dir.path());
    assert_eq!(body_text(&source, "", &[]), "");
    let _late = write_file(
        dir.path(),
        "counter_1001.db",
        &[entry("requests", "requests_total", &[], 3.0)],
    );
    assert_eq!(
        body_text(&source, "", &[]),
        "# HELP requests_total requests\n# TYPE requests_total counter\nrequests_total 3.0\n"
    );
}

#[tokio::test]
async fn multiprocess_answers_500_for_a_broken_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("counter_1001.db");
    fs::write(&path, b"garbage").unwrap();
    let app = make_metrics_app(source_of(dir.path()));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
