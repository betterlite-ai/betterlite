#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prometheus::collector_factory::collector::{Collector, MetricUpdate, MetricValue};
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::end_user::EndUserCostTrackingSettings;
use crate::fields::payload::StandardLoggingPayload;
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, ChildRemoval, RemovableMetric,
};
use crate::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use crate::prometheus_label_schema::PrometheusLabelSettings;
use crate::prometheus_latency::labels::LabelInputs;
use crate::prometheus_rate_limits::gauge::RateLimitGauges;
use crate::prometheus_rate_limits::metric::RateLimitMetric;
use crate::prometheus_rate_limits::{RemainingModelGroupUsage, RemainingUsageError};

pub(crate) type EventLog = Arc<Mutex<Vec<Recorded>>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Recorded {
    Update {
        metric: &'static str,
        labels: BTreeMap<String, String>,
        update: MetricUpdate,
    },
    Removal {
        metric: &'static str,
        label_values: Vec<Option<String>>,
    },
}

#[derive(Debug, thiserror::Error)]
#[error("the fake gauge refused the removal")]
pub(crate) struct RejectedRemoval;

#[derive(Clone, Debug)]
pub(crate) struct RecordingGauge {
    name: &'static str,
    labels: BTreeMap<String, String>,
    events: EventLog,
    fails_removal: bool,
}

impl Collector for RecordingGauge {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        Self {
            name: self.name,
            labels: labels
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            events: Arc::clone(&self.events),
            fails_removal: self.fails_removal,
        }
    }

    fn update(&self, update: MetricUpdate) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded::Update {
                metric: self.name,
                labels: self.labels.clone(),
                update,
            });
    }
}

impl RemovableMetric for RecordingGauge {
    type Error = RejectedRemoval;

    fn remove_child(
        &self,
        label_values: &[Option<String>],
    ) -> Result<ChildRemoval, RejectedRemoval> {
        if self.fails_removal {
            return Err(RejectedRemoval);
        }
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded::Removal {
                metric: self.name,
                label_values: label_values.to_vec(),
            });
        Ok(ChildRemoval::Removed)
    }
}

pub(crate) struct FakeGauges {
    events: EventLog,
    supported: BTreeMap<&'static str, Vec<String>>,
    tracker: BoundedPrometheusSeriesTracker,
    fails_removal: bool,
}

impl FakeGauges {
    pub(crate) fn with_labels(supported: &[(&'static str, &[&str])]) -> Self {
        let by_name = supported
            .iter()
            .map(|(name, labels)| {
                (
                    *name,
                    labels.iter().map(|label| (*label).to_owned()).collect(),
                )
            })
            .collect();
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            supported: by_name,
            tracker: BoundedPrometheusSeriesTracker::default(),
            fails_removal: false,
        }
    }

    pub(crate) fn failing_removals() -> Self {
        Self {
            fails_removal: true,
            ..Self::with_labels(&RATE_LIMIT_LABELS)
        }
    }

    pub(crate) fn events(&self) -> Vec<Recorded> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RateLimitGauges for FakeGauges {
    type Gauge = RecordingGauge;

    fn gauge(&self, metric: RateLimitMetric) -> RecordingGauge {
        RecordingGauge {
            name: metric.name(),
            labels: BTreeMap::new(),
            events: Arc::clone(&self.events),
            fails_removal: self.fails_removal,
        }
    }

    fn supported_labels(&self, metric: RateLimitMetric) -> &[String] {
        self.supported.get(metric.name()).map_or(&[], Vec::as_slice)
    }

    fn tracker(&self) -> &BoundedPrometheusSeriesTracker {
        &self.tracker
    }
}

pub(crate) const API_KEY_RATE_LIMIT_LABELS: &[&str] =
    &["hashed_api_key", "api_key_alias", "rate_limit_type"];
pub(crate) const TEAM_RATE_LIMIT_LABELS: &[&str] = &["team", "team_alias", "rate_limit_type"];
pub(crate) const LIMIT_FOR_MODEL_LABELS: &[&str] =
    &["hashed_api_key", "api_key_alias", "model", "model_id"];
pub(crate) const DEPLOYMENT_LIMIT_LABELS: &[&str] = &[
    "model_group",
    "litellm_model_name",
    "model_id",
    "api_base",
    "api_provider",
];
pub(crate) const REMAINING_LABELS: &[&str] = &[
    "model_group",
    "api_provider",
    "api_base",
    "litellm_model_name",
    "hashed_api_key",
    "api_key_alias",
    "model_id",
];

pub(crate) const RATE_LIMIT_LABELS: [(&str, &[&str]); 10] = [
    (
        "litellm_api_key_rate_limit_allowed_metric",
        API_KEY_RATE_LIMIT_LABELS,
    ),
    (
        "litellm_api_key_rate_limit_used_metric",
        API_KEY_RATE_LIMIT_LABELS,
    ),
    (
        "litellm_team_rate_limit_allowed_metric",
        TEAM_RATE_LIMIT_LABELS,
    ),
    (
        "litellm_team_rate_limit_used_metric",
        TEAM_RATE_LIMIT_LABELS,
    ),
    (
        "litellm_remaining_api_key_requests_for_model",
        LIMIT_FOR_MODEL_LABELS,
    ),
    (
        "litellm_remaining_api_key_tokens_for_model",
        LIMIT_FOR_MODEL_LABELS,
    ),
    ("litellm_deployment_tpm_limit", DEPLOYMENT_LIMIT_LABELS),
    ("litellm_deployment_rpm_limit", DEPLOYMENT_LIMIT_LABELS),
    ("litellm_remaining_requests_metric", REMAINING_LABELS),
    ("litellm_remaining_tokens_metric", REMAINING_LABELS),
];

pub(crate) struct Fixture {
    pub(crate) values: UserApiKeyLabelValues,
    pub(crate) context: PrometheusLabelFactoryContext,
    pub(crate) settings: PrometheusLabelSettings,
}

impl Fixture {
    pub(crate) fn new(pairs: &[(&str, &str)]) -> Self {
        let values = label_values(pairs);
        let settings = PrometheusLabelSettings::with_defaults();
        let context = PrometheusLabelFactoryContext::new(
            &values,
            &settings,
            EndUserCostTrackingSettings::default(),
        )
        .unwrap();
        Self {
            values,
            context,
            settings,
        }
    }

    pub(crate) fn inputs(&self) -> LabelInputs<'_> {
        LabelInputs {
            values: &self.values,
            settings: &self.settings,
            end_user_settings: EndUserCostTrackingSettings::default(),
            context: Some(&self.context),
        }
    }
}

pub(crate) fn label_values(pairs: &[(&str, &str)]) -> UserApiKeyLabelValues {
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

pub(crate) fn number(literal: &str) -> Json {
    Json::Number(literal.parse::<Number>().unwrap())
}

pub(crate) fn json_object(entries: &[(&str, Json)]) -> JsonObject {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
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
        ("response_cost", number("0")),
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

pub(crate) fn payload() -> StandardLoggingPayload {
    let text = serde_json::to_string(&Json::Object(base_document())).unwrap();
    serde_json::from_str(&text).unwrap()
}

pub(crate) fn with_headers(
    mut payload: StandardLoggingPayload,
    headers: &[(&str, Json)],
) -> StandardLoggingPayload {
    payload.hidden_params.additional_headers = Some(
        headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    );
    payload
}

pub(crate) fn with_model_group(
    mut payload: StandardLoggingPayload,
    group: &str,
) -> StandardLoggingPayload {
    payload.model_group = Some(group.to_owned());
    payload
}

pub(crate) fn with_team_alias(
    mut payload: StandardLoggingPayload,
    alias: &str,
) -> StandardLoggingPayload {
    payload.metadata.user_api_key_team_alias = Some(alias.to_owned());
    payload
}

pub(crate) fn set_event(metric: RateLimitMetric, labels: &[(&str, &str)], amount: i64) -> Recorded {
    Recorded::Update {
        metric: metric.name(),
        labels: labels
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        update: MetricUpdate::Set(MetricValue::Finite(Decimal::from(amount))),
    }
}

pub(crate) fn removal_event(metric: RateLimitMetric, label_values: &[Option<&str>]) -> Recorded {
    let mut owned: Vec<Option<String>> = Vec::with_capacity(label_values.len());
    for value in label_values {
        owned.push((*value).map(str::to_owned));
    }
    Recorded::Removal {
        metric: metric.name(),
        label_values: owned,
    }
}

pub(crate) struct ScriptedUsage {
    calls: Mutex<Vec<String>>,
    answer: BTreeMap<String, i64>,
    fails: bool,
}

impl ScriptedUsage {
    pub(crate) fn answering(entries: &[(&str, i64)]) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            answer: entries
                .iter()
                .map(|(name, value)| ((*name).to_owned(), *value))
                .collect(),
            fails: false,
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            answer: BTreeMap::new(),
            fails: true,
        }
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl RemainingModelGroupUsage for ScriptedUsage {
    async fn get_remaining_model_group_usage(
        &self,
        model_group: &str,
    ) -> Result<BTreeMap<String, i64>, RemainingUsageError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model_group.to_owned());
        if self.fails {
            return Err(RemainingUsageError::Unavailable);
        }
        Ok(self.answer.clone())
    }
}
