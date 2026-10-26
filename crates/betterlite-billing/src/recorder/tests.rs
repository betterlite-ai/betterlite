#![cfg(test)]

use std::time::Duration;

use opentelemetry_sdk::metrics::InMemoryMetricExporter;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
use rstest::rstest;

use super::{METER_NAME, METRIC_NAME, SHUTDOWN_FLUSH_TIMEOUT};
use crate::billable_category::BillableCategory;
use crate::tests::{in_memory, logged};

fn points(exporter: &InMemoryMetricExporter) -> Vec<(String, u64)> {
    let batches = exporter.get_finished_metrics().unwrap();
    let scope = batches.last().unwrap().scope_metrics().next().unwrap();
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
    sum.data_points()
        .map(|point| {
            let mut pairs: Vec<String> = point
                .attributes()
                .map(|pair| format!("{}={}", pair.key, pair.value))
                .collect();
            pairs.sort();
            (pairs.join(","), point.value())
        })
        .collect()
}

#[rstest]
#[case::llm_with_model(
    BillableCategory::Llm,
    "/v1/chat/completions",
    200,
    Some("model-a"),
    "http.response.status_code=200,http.route=/v1/chat/completions,litellm.endpoint.category=llm,litellm.model_id=model-a"
)]
#[case::mcp_without_model(
    BillableCategory::Mcp,
    "/mcp",
    201,
    None,
    "http.response.status_code=201,http.route=/mcp,litellm.endpoint.category=mcp"
)]
#[case::a2a_with_empty_model(
    BillableCategory::A2a,
    "/a2a/agent",
    204,
    Some(""),
    "http.response.status_code=204,http.route=/a2a/agent,litellm.endpoint.category=a2a"
)]
fn records_the_request_attributes(
    #[case] category: BillableCategory,
    #[case] route: &str,
    #[case] status_code: u16,
    #[case] model_id: Option<&str>,
    #[case] expected: &str,
) {
    let (recorder, exporter) = in_memory();
    recorder.record(category, route, status_code, model_id);
    recorder.shutdown();
    assert_eq!(points(&exporter), vec![(expected.to_owned(), 1)]);
}

#[test]
fn counts_each_repeated_request() {
    let (recorder, exporter) = in_memory();
    recorder.record(BillableCategory::Llm, "/v1/responses", 200, None);
    recorder.record(BillableCategory::Llm, "/v1/responses", 200, None);
    recorder.shutdown();
    let counted = points(&exporter);
    assert_eq!(counted.len(), 1);
    assert_eq!(counted[0].1, 2);
}

#[test]
fn shutdown_flushes_pending_counts_within_five_seconds() {
    assert_eq!(SHUTDOWN_FLUSH_TIMEOUT, Duration::from_secs(5));
    let (recorder, exporter) = in_memory();
    recorder.record(BillableCategory::Llm, "/v1/embeddings", 200, None);
    assert!(exporter.get_finished_metrics().unwrap().is_empty());
    let ((), output) = logged(|| recorder.shutdown());
    assert!(!output.contains("WARN"), "{output}");
    assert_eq!(points(&exporter).len(), 1);
}

#[test]
fn shutdown_warns_when_the_final_flush_fails() {
    let (recorder, _exporter) = in_memory();
    recorder.shutdown();
    let ((), output) = logged(|| recorder.shutdown());
    assert!(output.contains("WARN"), "{output}");
    assert!(
        output.contains("Billing metrics: final flush failed"),
        "{output}"
    );
}
