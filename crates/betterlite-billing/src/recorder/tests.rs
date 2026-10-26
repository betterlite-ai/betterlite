#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use rstest::rstest;
use tracing::subscriber::with_default;

use super::{BillingMetricsRecorder, METER_NAME, METRIC_NAME, SHUTDOWN_FLUSH_TIMEOUT};
use crate::billable_category::BillableCategory;

type Point = (BTreeMap<String, String>, u64);

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn logged<F>(action: F) -> String
where
    F: FnOnce(),
{
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    with_default(subscriber, action);
    String::from_utf8(captured.0.lock().unwrap().clone()).unwrap()
}

fn in_memory() -> (BillingMetricsRecorder, InMemoryMetricExporter) {
    let exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(exporter.clone()).build();
    let provider = SdkMeterProvider::builder().with_reader(reader).build();
    (BillingMetricsRecorder::new(provider), exporter)
}

fn points(exporter: &InMemoryMetricExporter) -> Vec<Point> {
    let batches = exporter.get_finished_metrics().unwrap();
    let resource_metrics = batches.last().unwrap();
    let scope = resource_metrics.scope_metrics().next().unwrap();
    assert_eq!(scope.scope().name(), METER_NAME);
    let metric = scope.metrics().next().unwrap();
    assert_eq!(metric.name(), METRIC_NAME);
    assert_eq!(metric.unit(), "{request}");
    assert_eq!(
        metric.description(),
        "Count of 2xx HTTP requests to billable LLM/MCP/A2A endpoints"
    );
    let AggregatedMetrics::U64(MetricData::Sum(sum)) = metric.data() else {
        panic!("the billable counter is not a u64 sum");
    };
    assert!(sum.is_monotonic());
    let mut collected: Vec<Point> = sum
        .data_points()
        .map(|point| {
            let attributes = point
                .attributes()
                .map(|pair| (pair.key.as_str().to_owned(), pair.value.to_string()))
                .collect();
            (attributes, point.value())
        })
        .collect();
    collected.sort();
    collected
}

fn attributes(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::llm_with_model(
    BillableCategory::Llm,
    "/v1/chat/completions",
    200,
    Some("model-a"),
    &[
        ("http.response.status_code", "200"),
        ("http.route", "/v1/chat/completions"),
        ("litellm.endpoint.category", "llm"),
        ("litellm.model_id", "model-a"),
    ]
)]
#[case::mcp_without_model(
    BillableCategory::Mcp,
    "/mcp",
    201,
    None,
    &[
        ("http.response.status_code", "201"),
        ("http.route", "/mcp"),
        ("litellm.endpoint.category", "mcp"),
    ]
)]
#[case::a2a_with_empty_model(
    BillableCategory::A2a,
    "/a2a/agent",
    204,
    Some(""),
    &[
        ("http.response.status_code", "204"),
        ("http.route", "/a2a/agent"),
        ("litellm.endpoint.category", "a2a"),
    ]
)]
fn records_the_request_attributes(
    #[case] category: BillableCategory,
    #[case] route: &str,
    #[case] status_code: u16,
    #[case] model_id: Option<&str>,
    #[case] expected: &[(&str, &str)],
) {
    let (recorder, exporter) = in_memory();
    recorder.record(category, route, status_code, model_id);
    recorder.shutdown();
    assert_eq!(points(&exporter), vec![(attributes(expected), 1)]);
}

#[test]
fn counts_each_repeated_request() {
    let (recorder, exporter) = in_memory();
    recorder.record(BillableCategory::Llm, "/v1/responses", 200, None);
    recorder.record(BillableCategory::Llm, "/v1/responses", 200, None);
    recorder.record(BillableCategory::Llm, "/v1/responses", 200, None);
    recorder.shutdown();
    let counted = points(&exporter);
    assert_eq!(counted.len(), 1);
    assert_eq!(counted[0].1, 3);
}

#[test]
fn shutdown_flushes_pending_counts_within_five_seconds() {
    assert_eq!(SHUTDOWN_FLUSH_TIMEOUT, Duration::from_secs(5));
    let (recorder, exporter) = in_memory();
    recorder.record(BillableCategory::Llm, "/v1/embeddings", 200, None);
    assert!(exporter.get_finished_metrics().unwrap().is_empty());
    let output = logged(|| recorder.shutdown());
    assert_eq!(output, "");
    assert_eq!(points(&exporter).len(), 1);
}

#[test]
fn shutdown_warns_when_the_final_flush_fails() {
    let (recorder, _exporter) = in_memory();
    recorder.shutdown();
    let output = logged(|| recorder.shutdown());
    assert!(output.contains("WARN"), "{output}");
    assert!(
        output.contains("Billing metrics: final flush failed"),
        "{output}"
    );
}
