use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use betterlite_models::decisions::Json;
use betterlite_models::zero_cost::{ZeroCostDiagnostic, ZeroCostReason};
use betterlite_prometheus::collector_factory::collector::{Collector, MetricUpdate, MetricValue};
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::end_user::EndUserIdError;
use crate::fields::payload::StandardLoggingPayload;
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, ChildRemoval, RemovableMetric, SeriesLimits,
};
use crate::prometheus_label_projection::end_user_resolution::EndUserResolver;
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_schema::PrometheusLabelSettings;
use crate::prometheus_usage::counters::{UsageCounters, UsageLabels};
use crate::prometheus_usage::metric::UsageMetric;
use crate::prometheus_usage::request_spend::{
    increment_request_and_spend, increment_zero_cost_requests,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Recorded {
    Increment {
        metric: UsageMetric,
        labels: BTreeMap<String, String>,
        amount: MetricValue,
    },
    Set {
        metric: UsageMetric,
        labels: BTreeMap<String, String>,
        amount: MetricValue,
    },
    Removal {
        metric: UsageMetric,
    },
}

#[derive(Debug, Default)]
pub(super) struct FakeLog {
    events: Vec<Recorded>,
}

#[derive(Debug, thiserror::Error)]
#[error("the fake metric refused the removal")]
pub(super) struct FakeError;

#[derive(Clone, Debug)]
pub(super) struct FakeMetric {
    metric: UsageMetric,
    labels: BTreeMap<String, String>,
    log: Arc<Mutex<FakeLog>>,
}

impl Collector for FakeMetric {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        let mut child = self.clone();
        child.labels = labels
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        child
    }

    fn update(&self, update: MetricUpdate) {
        let event = match update {
            MetricUpdate::Inc(amount) => Recorded::Increment {
                metric: self.metric,
                labels: self.labels.clone(),
                amount,
            },
            MetricUpdate::Set(amount) => Recorded::Set {
                metric: self.metric,
                labels: self.labels.clone(),
                amount,
            },
            MetricUpdate::Observe(_) | MetricUpdate::Remove => return,
        };
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .events
            .push(event);
    }
}

impl RemovableMetric for FakeMetric {
    type Error = FakeError;

    fn remove_child(&self, _label_values: &[Option<String>]) -> Result<ChildRemoval, Self::Error> {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .events
            .push(Recorded::Removal {
                metric: self.metric,
            });
        Ok(ChildRemoval::Removed)
    }
}

pub(super) struct FakeCounters {
    log: Arc<Mutex<FakeLog>>,
    supported: BTreeMap<String, Vec<String>>,
    tracker: BoundedPrometheusSeriesTracker,
    limits: SeriesLimits,
}

impl FakeCounters {
    pub(super) fn with_labels(supported: &[(&str, &[&str])]) -> Self {
        let mut map = BTreeMap::new();
        for (metric, labels) in supported {
            map.insert(
                (*metric).to_owned(),
                labels.iter().map(|label| (*label).to_owned()).collect(),
            );
        }
        Self {
            log: Arc::new(Mutex::new(FakeLog::default())),
            supported: map,
            tracker: BoundedPrometheusSeriesTracker::default(),
            limits: SeriesLimits::default(),
        }
    }

    pub(super) fn with_limits(mut self, limits: SeriesLimits) -> Self {
        self.limits = limits;
        self
    }

    pub(super) fn events(&self) -> Vec<Recorded> {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .events
            .clone()
    }
}

impl UsageCounters for FakeCounters {
    type Counter = FakeMetric;

    fn counter(&self, metric: UsageMetric) -> FakeMetric {
        FakeMetric {
            metric,
            labels: BTreeMap::new(),
            log: Arc::clone(&self.log),
        }
    }

    fn supported_labels(&self, metric: UsageMetric) -> &[String] {
        self.supported.get(metric.name()).map_or(&[], Vec::as_slice)
    }

    fn tracker(&self) -> &BoundedPrometheusSeriesTracker {
        &self.tracker
    }

    fn limits(&self) -> &SeriesLimits {
        &self.limits
    }
}

struct EchoResolver;

impl EndUserResolver for EchoResolver {
    fn resolve_end_user(&self, end_user: Option<&str>) -> Result<Option<String>, EndUserIdError> {
        Ok(end_user.map(ToOwned::to_owned))
    }
}

pub(super) struct Fixture {
    values: UserApiKeyLabelValues,
    context: PrometheusLabelFactoryContext,
    pub(super) settings: PrometheusLabelSettings,
}

impl Fixture {
    pub(super) fn new(pairs: &[(&str, &str)]) -> Self {
        let values = label_values(pairs);
        let settings = PrometheusLabelSettings::with_defaults();
        let context = PrometheusLabelFactoryContext::with_resolver(
            &values,
            &settings,
            Arc::new(EchoResolver),
        )
        .unwrap();
        Self {
            values,
            context,
            settings,
        }
    }

    pub(super) fn labels(&self) -> UsageLabels<'_> {
        UsageLabels {
            values: &self.values,
            context: &self.context,
            settings: &self.settings,
        }
    }

    pub(super) fn with_end_user(end_user: &str) -> Self {
        Self::new(&[("model", "gpt-4o"), ("end_user", end_user)])
    }
}

pub(super) fn label_values(pairs: &[(&str, &str)]) -> UserApiKeyLabelValues {
    let entries: Vec<(String, LabelDumpValue)> = pairs
        .iter()
        .map(|(name, value)| {
            (
                (*name).to_owned(),
                LabelDumpValue::String((*value).to_owned()),
            )
        })
        .collect();
    UserApiKeyLabelValues::from_entries(&entries)
}

pub(super) fn number(literal: &str) -> Json {
    Json::Number(literal.parse::<Number>().unwrap())
}

pub(super) fn object(entries: Vec<(&str, Json)>) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

pub(super) fn finite(literal: &str) -> MetricValue {
    MetricValue::Finite(literal.parse().unwrap())
}

pub(super) fn one() -> MetricValue {
    MetricValue::Finite(Decimal::ONE)
}

pub(super) fn increments(counters: &FakeCounters) -> Vec<(UsageMetric, MetricValue)> {
    counters
        .events()
        .into_iter()
        .filter_map(|event| match event {
            Recorded::Increment { metric, amount, .. } => Some((metric, amount)),
            Recorded::Set { .. } | Recorded::Removal { .. } => None,
        })
        .collect()
}

fn status_fields() -> Json {
    Json::Object(IndexMap::from([
        (
            "llm_api_status".to_owned(),
            Json::Text("success".to_owned()),
        ),
        (
            "guardrail_status".to_owned(),
            Json::Text("not_run".to_owned()),
        ),
    ]))
}

fn base_document() -> IndexMap<String, Json> {
    let entries = [
        ("id", Json::Text("payload".to_owned())),
        ("trace_id", Json::Text("trace".to_owned())),
        ("session_id", Json::Text("session".to_owned())),
        ("call_type", Json::Text("completion".to_owned())),
        ("status", Json::Text("success".to_owned())),
        ("status_fields", status_fields()),
        ("startTime", number("0")),
        ("endTime", number("0")),
        ("completionStartTime", number("0")),
        ("response_time", number("0")),
        ("model_map_information", Json::Object(IndexMap::new())),
        ("model", Json::Text("gpt-4o".to_owned())),
        ("api_base", Json::Text(String::new())),
        ("metadata", Json::Object(IndexMap::new())),
        ("saved_cache_cost", number("0")),
        ("request_tags", Json::List(Vec::new())),
        ("model_parameters", Json::Object(IndexMap::new())),
        ("hidden_params", Json::Object(IndexMap::new())),
        ("total_tokens", number("0")),
        ("prompt_tokens", number("0")),
        ("completion_tokens", number("0")),
    ];
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

pub(super) fn payload(response_cost: Decimal) -> StandardLoggingPayload {
    let mut fields = base_document();
    fields.insert(
        "response_cost".to_owned(),
        number(&response_cost.to_string()),
    );
    let text = serde_json::to_string(&Json::Object(fields)).unwrap();
    serde_json::from_str(&text).unwrap()
}

pub(super) fn with_usage(
    mut payload: StandardLoggingPayload,
    usage: Json,
) -> StandardLoggingPayload {
    payload.metadata.usage_object = Some(usage);
    payload
}

pub(super) fn mcp_payload(metadata: Option<Json>) -> StandardLoggingPayload {
    let mut payload = payload(Decimal::ZERO);
    payload.metadata.mcp_tool_call_metadata = metadata;
    payload
}

pub(super) fn diagnostic() -> ZeroCostDiagnostic {
    ZeroCostDiagnostic {
        reason: ZeroCostReason::MissingPricingKey,
        pricing_model: "gpt-4o".to_owned(),
        missing_pricing_keys: Vec::new(),
    }
}

pub(super) fn emit_zero_cost(counters: &FakeCounters, end_users: &[&str]) {
    for end_user in end_users {
        let fixture = Fixture::with_end_user(end_user);
        increment_zero_cost_requests(counters, Some(&diagnostic()), fixture.labels()).unwrap();
    }
}

pub(super) fn emit_requests(counters: &FakeCounters, end_users: &[&str]) {
    for end_user in end_users {
        let fixture = Fixture::with_end_user(end_user);
        increment_request_and_spend(
            counters,
            &payload(Decimal::ZERO),
            fixture.labels(),
            Instant::now(),
        )
        .unwrap();
    }
}

pub(super) const REQUEST_LABELS: [(&str, &[&str]); 2] = [
    ("litellm_requests_metric", &["model", "end_user"]),
    ("litellm_spend_metric", &["model", "end_user"]),
];

pub(super) const ZERO_COST_LABELS: [(&str, &[&str]); 1] = [(
    "litellm_zero_cost_requests_total",
    &["model", "end_user", "reason"],
)];

pub(super) const MCP_LABELS: [(&str, &[&str]); 2] = [
    (
        "litellm_mcp_tool_calls_total",
        &[
            "model",
            "mcp_tool_name",
            "mcp_server_name",
            "team",
            "end_user",
        ],
    ),
    (
        "litellm_mcp_tool_call_spend_metric",
        &[
            "model",
            "mcp_tool_name",
            "mcp_server_name",
            "team",
            "end_user",
        ],
    ),
];

pub(super) trait DecimalFromMetric {
    fn decimal(self) -> Decimal;
}

impl DecimalFromMetric for MetricValue {
    fn decimal(self) -> Decimal {
        match self {
            Self::Finite(amount) => amount,
            Self::PositiveInfinity | Self::NegativeInfinity | Self::NotANumber => {
                panic!("the fixture is not a finite amount")
            }
        }
    }
}
