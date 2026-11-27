use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::fields::payload::records::ErrorInformation;
use betterlite_logging::prometheus::budget_lookups::lists::BudgetTableSnapshot;
use betterlite_logging::prometheus::budget_lookups::lookups::{
    BudgetLookupError, BudgetLookups, UserLookupOptions,
};
use betterlite_logging::prometheus::budget_lookups::records::{
    CachedEndUser, KeyRecord, OrgRecord, TeamRecord, UserRecord,
};
use betterlite_logging::prometheus_exporter::{PrometheusCollectors, PrometheusExporterSettings};
use betterlite_logging::prometheus_helpers::bounded_series_tracker::SeriesLimits;
use betterlite_models::decisions::Json;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use betterlite_prometheus::collector_registry::CollectorRegistry;
use betterlite_prometheus::collector_registry::metric::sample::Sample;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::prometheus_hooks::{
    BudgetLookupPort, PrometheusHookPorts, PrometheusHookSettings, PrometheusHooks,
};
use crate::prometheus_remaining_budget::fan_out::RemainingBudgetFanOut;

pub(crate) const REQUESTS: &str = "litellm_requests_metric";
pub(crate) const SPEND: &str = "litellm_spend_metric";
pub(crate) const FAILED: &str = "litellm_llm_api_failed_requests_metric";
pub(crate) const PROXY_FAILED: &str = "litellm_proxy_failed_requests_metric";
pub(crate) const PROXY_TOTAL: &str = "litellm_proxy_total_requests_metric";
pub(crate) const VIRTUAL_KEY_REQUESTS: &str = "litellm_remaining_api_key_requests_for_model";
pub(crate) const ORG_BUDGET: &str = "litellm_remaining_org_budget_metric";
pub(crate) const LATENCY: &str = "litellm_llm_api_latency_metric";
pub(crate) const DEPLOYMENT_FAILURES: &str = "litellm_deployment_failure_responses";
pub(crate) const DEPLOYMENT_SUCCESSES: &str = "litellm_deployment_success_responses";

// the cache and database reads the fan-out makes, recorded so a test can see which ids reached them
pub(crate) struct RecordingLookups {
    org_record: Mutex<Option<OrgRecord>>,
    keys: Mutex<Vec<String>>,
    orgs: Mutex<Vec<String>>,
}

impl RecordingLookups {
    fn new() -> Self {
        Self {
            org_record: Mutex::new(None),
            keys: Mutex::new(Vec::new()),
            orgs: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn keys(&self) -> Vec<String> {
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn orgs(&self) -> Vec<String> {
        self.orgs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn set_org_record(&self, record: OrgRecord) {
        *self
            .org_record
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(record);
    }
}

#[async_trait]
impl BudgetLookups for RecordingLookups {
    fn database_configured(&self) -> bool {
        true
    }

    async fn team_info(&self, _team_id: &str) -> Result<TeamRecord, BudgetLookupError> {
        Err(BudgetLookupError {
            message: "no team store in the fixture".to_owned(),
        })
    }

    async fn org_info(
        &self,
        org_id: &str,
        _include_budget_table: bool,
    ) -> Result<Option<OrgRecord>, BudgetLookupError> {
        self.orgs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(org_id.to_owned());
        let record = self
            .org_record
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(record)
    }

    async fn key_info(
        &self,
        hashed_token: &str,
        _check_cache_only: bool,
    ) -> Result<KeyRecord, BudgetLookupError> {
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(hashed_token.to_owned());
        Ok(KeyRecord {
            budget_reset_at: None,
        })
    }

    async fn user_info(
        &self,
        _user_id: &str,
        _options: UserLookupOptions,
    ) -> Result<Option<UserRecord>, BudgetLookupError> {
        Ok(None)
    }

    async fn cached_customer(
        &self,
        _end_user_id: &str,
    ) -> Result<Option<CachedEndUser>, BudgetLookupError> {
        Ok(None)
    }

    async fn default_customer_budget(
        &self,
        _budget_id: &str,
    ) -> Result<Option<BudgetTableSnapshot>, BudgetLookupError> {
        Ok(None)
    }
}

pub(crate) struct Fixture {
    pub(crate) hooks: PrometheusHooks,
    pub(crate) registry: CollectorRegistry,
    pub(crate) lookups: Arc<RecordingLookups>,
}

pub(crate) fn fixture() -> Fixture {
    fixture_with_settings(PrometheusHookSettings::default())
}

pub(crate) fn fixture_with_limits(limits: SeriesLimits) -> Fixture {
    let mut settings = PrometheusHookSettings::default();
    settings.limits = limits;
    fixture_with_settings(settings)
}

pub(crate) fn fixture_with_settings(settings: PrometheusHookSettings) -> Fixture {
    let registry = CollectorRegistry::default();
    let collectors = Arc::new(
        PrometheusCollectors::register(
            &PrometheusExporterSettings::default(),
            &settings.labels,
            &registry,
        )
        .expect("the collectors register"),
    );
    let lookups = Arc::new(RecordingLookups::new());
    let port = Arc::new(BudgetLookupPort::new(
        Arc::clone(&lookups),
        Arc::clone(&collectors),
        settings.labels.clone(),
        settings.limits,
        settings.end_user,
    ));
    let budget =
        RemainingBudgetFanOut::new(Arc::clone(&port), Duration::from_secs(1), &port.gauges);
    let hooks = PrometheusHooks::new(
        collectors,
        settings,
        PrometheusHookPorts::default(),
        budget,
        port,
    );
    Fixture {
        hooks,
        registry,
        lookups,
    }
}

pub(crate) fn dec(literal: &str) -> Decimal {
    literal.parse().unwrap()
}

pub(crate) fn event(payload: StandardLoggingPayload) -> LoggerEvent {
    event_with_metadata(payload, IndexMap::new())
}

pub(crate) fn event_with_metadata(
    payload: StandardLoggingPayload,
    metadata: IndexMap<String, Json>,
) -> LoggerEvent {
    LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Null),
        litellm_metadata: Arc::new(metadata),
        stream: false,
        guardrail_snapshot: None,
    }
}

pub(crate) fn error_information(message: &str) -> ErrorInformation {
    ErrorInformation {
        error_code: None,
        error_class: Some(Some("BadRequestError".to_owned())),
        llm_provider: None,
        traceback: None,
        error_message: Some(Some(message.to_owned())),
        error_provider_request_id: None,
        error_rate_limit_category: None,
        error_rate_limit_type: None,
        error_budget_entity_type: None,
        error_budget_entity_id: None,
        normalized_error: None,
        error_budget_limit: None,
        error_budget_spend: None,
    }
}

fn series(registry: &CollectorRegistry, metric: &str) -> Vec<(Vec<String>, Sample)> {
    registry
        .get(metric)
        .map_or_else(Vec::new, |collector| collector.series_in_use_order())
}

pub(crate) fn totals(registry: &CollectorRegistry, metric: &str) -> Vec<Decimal> {
    series(registry, metric)
        .iter()
        .filter_map(|(_labels, sample)| match sample.total {
            MetricValue::Finite(amount) => Some(amount),
            MetricValue::PositiveInfinity
            | MetricValue::NegativeInfinity
            | MetricValue::NotANumber => None,
        })
        .collect()
}

pub(crate) fn observations(registry: &CollectorRegistry, metric: &str) -> Vec<u64> {
    series(registry, metric)
        .iter()
        .map(|(_labels, sample)| sample.count)
        .collect()
}

pub(crate) fn labels_seen(registry: &CollectorRegistry, metric: &str) -> Vec<String> {
    series(registry, metric)
        .into_iter()
        .flat_map(|(labels, _sample)| labels)
        .collect()
}

fn number(literal: &str) -> Json {
    Json::Number(literal.parse::<Number>().unwrap())
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

pub(crate) fn payload(response_cost: Decimal) -> StandardLoggingPayload {
    let mut fields = base_document();
    fields.insert(
        "response_cost".to_owned(),
        number(&response_cost.to_string()),
    );
    let text = serde_json::to_string(&Json::Object(fields)).unwrap();
    serde_json::from_str(&text).unwrap()
}
