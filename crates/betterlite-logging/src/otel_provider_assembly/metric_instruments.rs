use opentelemetry::metrics::{Histogram, Meter};

use crate::otel::vocabulary::constants::attribute_keys::metric;

pub struct MetricInstruments {
    pub operation_duration: Histogram<f64>,
    pub token_usage: Histogram<f64>,
    pub usage_in_usd: Histogram<f64>,
    pub time_to_first_token: Histogram<f64>,
    pub time_per_output_token: Histogram<f64>,
    pub response_duration: Histogram<f64>,
}

fn histogram(
    meter: &Meter,
    name: &'static str,
    description: &'static str,
    unit: &'static str,
) -> Histogram<f64> {
    meter
        .f64_histogram(name)
        .with_description(description)
        .with_unit(unit)
        .build()
}

#[must_use]
pub fn create_instruments(meter: &Meter) -> MetricInstruments {
    MetricInstruments {
        operation_duration: histogram(
            meter,
            metric::OPERATION_DURATION,
            "GenAI operation duration",
            "s",
        ),
        token_usage: histogram(meter, metric::TOKEN_USAGE, "GenAI token usage", "{token}"),
        usage_in_usd: histogram(meter, metric::TOKEN_COST, "GenAI request cost", "USD"),
        time_to_first_token: histogram(
            meter,
            metric::TIME_TO_FIRST_TOKEN,
            "Time to first token for streaming requests",
            "s",
        ),
        time_per_output_token: histogram(
            meter,
            metric::TIME_PER_OUTPUT_TOKEN,
            "Average time per output token (generation time / completion tokens)",
            "s",
        ),
        response_duration: histogram(
            meter,
            metric::RESPONSE_DURATION,
            "Total LLM API generation time (excludes LiteLLM overhead)",
            "s",
        ),
    }
}

#[path = "metric_instruments/tests.rs"]
mod tests;
