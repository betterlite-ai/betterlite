use std::collections::BTreeMap;

use betterlite_logging::otel_provider_assembly::metric_instruments::create_instruments;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::data::{ResourceMetrics, ScopeMetrics};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

type Histograms = BTreeMap<String, (String, String)>;

#[test]
fn the_six_histograms_carry_the_litellm_name_description_and_unit() {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    let instruments = create_instruments(&provider.meter("genai-metrics-create"));
    instruments.operation_duration.record(1.0_f64, &[]);
    instruments.token_usage.record(1.0_f64, &[]);
    instruments.usage_in_usd.record(1.0_f64, &[]);
    instruments.time_to_first_token.record(1.0_f64, &[]);
    instruments.time_per_output_token.record(1.0_f64, &[]);
    instruments.response_duration.record(1.0_f64, &[]);
    provider.force_flush().expect("flush");
    assert_eq!(described(&exporter), expected());
}

fn described(exporter: &InMemoryMetricExporter) -> Histograms {
    exporter
        .get_finished_metrics()
        .expect("finished metrics")
        .iter()
        .flat_map(ResourceMetrics::scope_metrics)
        .flat_map(ScopeMetrics::metrics)
        .map(|metric| {
            (
                metric.name().to_owned(),
                (metric.description().to_owned(), metric.unit().to_owned()),
            )
        })
        .collect()
}

fn expected() -> Histograms {
    [
        (
            "gen_ai.client.operation.duration",
            ("GenAI operation duration", "s"),
        ),
        (
            "gen_ai.client.response.duration",
            (
                "Total LLM API generation time (excludes LiteLLM overhead)",
                "s",
            ),
        ),
        (
            "gen_ai.client.token.usage",
            ("GenAI token usage", "{token}"),
        ),
        (
            "gen_ai.server.time_per_output_token",
            (
                "Average time per output token (generation time / completion tokens)",
                "s",
            ),
        ),
        (
            "gen_ai.server.time_to_first_token",
            ("Time to first token for streaming requests", "s"),
        ),
        ("gen_ai.usage.cost", ("GenAI request cost", "USD")),
    ]
    .into_iter()
    .map(|(name, (description, unit))| (name.to_owned(), (description.to_owned(), unit.to_owned())))
    .collect()
}
