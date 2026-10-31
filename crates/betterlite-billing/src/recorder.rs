use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, MeterProvider as _};
use opentelemetry_sdk::metrics::SdkMeterProvider;

use crate::billable_category::BillableCategory;

mod tests;

pub const METRIC_NAME: &str = "litellm.enterprise.billable_requests";
pub const METER_NAME: &str = "litellm.enterprise.billing";
pub const SHUTDOWN_FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct BillingMetricsRecorder {
    provider: SdkMeterProvider,
    counter: Counter<u64>,
}

impl BillingMetricsRecorder {
    #[must_use]
    pub fn new(provider: SdkMeterProvider) -> Self {
        let counter = provider
            .meter(METER_NAME)
            .u64_counter(METRIC_NAME)
            .with_unit("{request}")
            .with_description("Count of 2xx HTTP requests to billable LLM/MCP/A2A endpoints")
            .build();
        Self { provider, counter }
    }

    pub fn record(
        &self,
        category: BillableCategory,
        route: &str,
        status_code: u16,
        model_id: Option<&str>,
    ) {
        let model = model_id
            .filter(|model| !model.is_empty())
            .map(|model| KeyValue::new("litellm.model_id", model.to_owned()));
        let attributes: Vec<KeyValue> = [
            KeyValue::new("litellm.endpoint.category", category.as_str()),
            KeyValue::new("http.route", route.to_owned()),
            KeyValue::new("http.response.status_code", i64::from(status_code)),
        ]
        .into_iter()
        .chain(model)
        .collect();
        self.counter.add(1, &attributes);
    }

    // without the final flush up to one export interval of billable counts is lost on every restart
    pub fn shutdown(&self) {
        if let Err(error) = self.provider.shutdown_with_timeout(SHUTDOWN_FLUSH_TIMEOUT) {
            tracing::warn!("Billing metrics: final flush failed: {error}");
        }
    }
}
