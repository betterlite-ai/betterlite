#![cfg(test)]
use opentelemetry_sdk::metrics::Temporality;
use opentelemetry_sdk::metrics::exporter::PushMetricExporter as _;

use super::MetricSink;

#[test]
fn metric_sink_temporality_follows_the_configured_exporter() {
    let exporter = opentelemetry_stdout::MetricExporter::builder()
        .with_temporality(Temporality::Delta)
        .build();
    let sink = MetricSink::Console(Box::new(exporter));
    assert_eq!(sink.temporality(), Temporality::Delta);
}
