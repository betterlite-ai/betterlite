use serde::Serialize;

// DatadogMetricSeries.type: 1 counts, 3 gauges; the Datadog series API takes the number, not its name
pub const COUNT_TYPE: u8 = 1;
pub const GAUGE_TYPE: u8 = 3;

pub const TOTAL_LATENCY_METRIC: &str = "litellm.request.total_latency";
pub const PROVIDER_LATENCY_METRIC: &str = "litellm.llm_api.latency";
pub const OVERHEAD_LATENCY_METRIC: &str = "litellm.overhead.latency";
pub const REQUEST_COUNT_METRIC: &str = "litellm.llm_api.request_count";
pub const HEALTH_CHECK_METRIC: &str = "litellm.health_check";
pub const HEALTH_CHECK_TAG: &str = "env:health_check";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DatadogMetricPoint {
    pub timestamp: i64,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DatadogMetricSeries {
    pub metric: &'static str,
    #[serde(rename = "type")]
    pub kind: u8,
    pub points: Vec<DatadogMetricPoint>,
    pub tags: Vec<String>,
    // count and rate series need an interval; the gauge series LiteLLM builds carry no key at all
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DatadogMetricsPayload {
    pub series: Vec<DatadogMetricSeries>,
}
