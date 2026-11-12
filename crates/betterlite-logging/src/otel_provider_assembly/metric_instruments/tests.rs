#![cfg(test)]

use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::data::{ResourceMetrics, ScopeMetrics};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use rstest::rstest;

use crate::otel_provider_assembly::metric_instruments::create_instruments;

type Described = (String, String, String);

fn recorded_instruments() -> Vec<Described> {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    let instruments = create_instruments(&provider.meter("instrument-test"));
    instruments.operation_duration.record(1.0_f64, &[]);
    instruments.token_usage.record(1.0_f64, &[]);
    instruments.usage_in_usd.record(1.0_f64, &[]);
    instruments.time_to_first_token.record(1.0_f64, &[]);
    instruments.time_per_output_token.record(1.0_f64, &[]);
    instruments.response_duration.record(1.0_f64, &[]);
    provider.force_flush().expect("flush");
    let finished = exporter.get_finished_metrics().expect("finished metrics");
    let mut described: Vec<Described> = finished
        .iter()
        .flat_map(ResourceMetrics::scope_metrics)
        .flat_map(ScopeMetrics::metrics)
        .map(|metric| {
            (
                metric.name().to_owned(),
                metric.description().to_owned(),
                metric.unit().to_owned(),
            )
        })
        .collect();
    described.sort();
    described
}

#[rstest]
#[case::operation_duration("gen_ai.client.operation.duration", "GenAI operation duration", "s")]
#[case::token_usage("gen_ai.client.token.usage", "GenAI token usage", "{token}")]
#[case::cost("gen_ai.usage.cost", "GenAI request cost", "USD")]
#[case::time_to_first_token(
    "gen_ai.server.time_to_first_token",
    "Time to first token for streaming requests",
    "s"
)]
#[case::time_per_output_token(
    "gen_ai.server.time_per_output_token",
    "Average time per output token (generation time / completion tokens)",
    "s"
)]
#[case::response_duration(
    "gen_ai.client.response.duration",
    "Total LLM API generation time (excludes LiteLLM overhead)",
    "s"
)]
fn each_histogram_has_the_litellm_name_description_and_unit(
    #[case] name: &str,
    #[case] description: &str,
    #[case] unit: &str,
) {
    let described = recorded_instruments();
    assert_eq!(described.len(), 6);
    assert!(described.contains(&(name.to_owned(), description.to_owned(), unit.to_owned())));
}
