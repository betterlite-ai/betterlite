use std::convert::Infallible;
use std::error::Error as StdError;

use betterlite_prometheus::collector_factory::collector::{Collector, MetricValue};
use rust_decimal::Decimal;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardrailStatus {
    Success,
    Error,
    Intervened,
}

impl GuardrailStatus {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
            Self::Intervened => "intervened",
        }
    }
}

// the three guardrail series bypass the label factory; a collector that rejects a label set reports it here, and record_guardrail_metrics catches it
pub trait GuardrailSeries {
    type Error: StdError + 'static;

    fn observe(&self, labels: &[(&str, &str)], seconds: Decimal) -> Result<(), Self::Error>;
    fn increment(&self, labels: &[(&str, &str)]) -> Result<(), Self::Error>;
}

impl<Metric: Collector> GuardrailSeries for Metric {
    type Error = Infallible;

    fn observe(&self, labels: &[(&str, &str)], seconds: Decimal) -> Result<(), Self::Error> {
        self.labels(labels).observe(MetricValue::Finite(seconds));
        Ok(())
    }

    fn increment(&self, labels: &[(&str, &str)]) -> Result<(), Self::Error> {
        self.labels(labels).inc(MetricValue::Finite(Decimal::ONE));
        Ok(())
    }
}

pub struct GuardrailCollectors<'collectors, Error> {
    pub latency: &'collectors dyn GuardrailSeries<Error = Error>,
    pub requests: &'collectors dyn GuardrailSeries<Error = Error>,
    pub errors: &'collectors dyn GuardrailSeries<Error = Error>,
}

#[derive(Clone, Copy, Debug)]
pub struct GuardrailObservation<'observation> {
    pub guardrail_name: &'observation str,
    pub latency_seconds: Decimal,
    pub status: GuardrailStatus,
    pub error_type: Option<&'observation str>,
    pub hook_type: &'observation str,
}

pub fn record_guardrail_metrics<Error>(
    collectors: &GuardrailCollectors<'_, Error>,
    observation: &GuardrailObservation<'_>,
) where
    Error: StdError + 'static,
{
    if let Err(error) = record_series(collectors, observation) {
        tracing::debug!("Error recording guardrail metrics: {error}");
    }
}

fn record_series<Error>(
    collectors: &GuardrailCollectors<'_, Error>,
    observation: &GuardrailObservation<'_>,
) -> Result<(), Error>
where
    Error: StdError + 'static,
{
    let status = observation.status.as_str();
    let histogram_error_type = observation
        .error_type
        .filter(|text| !text.is_empty())
        .unwrap_or("none");
    let latency_labels = [
        ("guardrail_name", observation.guardrail_name),
        ("status", status),
        ("error_type", histogram_error_type),
        ("hook_type", observation.hook_type),
    ];
    collectors
        .latency
        .observe(&latency_labels, observation.latency_seconds)?;
    let request_labels = [
        ("guardrail_name", observation.guardrail_name),
        ("status", status),
        ("hook_type", observation.hook_type),
    ];
    collectors.requests.increment(&request_labels)?;
    if observation.status == GuardrailStatus::Error
        && let Some(counter_error_type) = observation.error_type.filter(|text| !text.is_empty())
    {
        let error_labels = [
            ("guardrail_name", observation.guardrail_name),
            ("error_type", counter_error_type),
            ("hook_type", observation.hook_type),
        ];
        collectors.errors.increment(&error_labels)?;
    }
    Ok(())
}

#[path = "tests/guardrail_series.rs"]
mod tests;
