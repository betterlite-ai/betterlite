use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Serialize;

// the Metric API caps attribute values, so caller-controlled model strings cannot inflate the shared batch payload
pub const ATTRIBUTE_MAX_LEN: usize = 255;

pub const METRIC_REQUESTS: &str = "litellm.requests";
pub const METRIC_COST_USD: &str = "litellm.cost.usd";
pub const METRIC_PROMPT_TOKENS: &str = "litellm.tokens.prompt";
pub const METRIC_COMPLETION_TOKENS: &str = "litellm.tokens.completion";
pub const METRIC_TOTAL_TOKENS: &str = "litellm.tokens.total";
pub const METRIC_REQUEST_DURATION_MS: &str = "litellm.request.duration_ms";
pub const METRIC_TEAM_MAX_BUDGET: &str = "litellm.team.max_budget";
pub const METRIC_TEAM_REMAINING_BUDGET: &str = "litellm.team.remaining_budget";

pub type MetricAttributes = IndexMap<String, String>;

#[derive(Debug, Serialize)]
pub struct NewRelicSummaryValue {
    pub count: usize,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum CountValue {
    Float(f64),
    Decimal(#[serde(with = "rust_decimal::serde::arbitrary_precision")] Decimal),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NewRelicMetric {
    Count {
        name: &'static str,
        value: CountValue,
        attributes: MetricAttributes,
    },
    Gauge {
        name: &'static str,
        #[serde(with = "rust_decimal::serde::arbitrary_precision")]
        value: Decimal,
        attributes: MetricAttributes,
    },
    Summary {
        name: &'static str,
        value: NewRelicSummaryValue,
        attributes: MetricAttributes,
    },
}

#[derive(Debug, Serialize)]
pub struct NewRelicMetricCommon {
    pub timestamp: u64,
    #[serde(rename = "interval.ms")]
    pub interval_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct NewRelicMetricEnvelope {
    pub common: NewRelicMetricCommon,
    pub metrics: Vec<NewRelicMetric>,
}
