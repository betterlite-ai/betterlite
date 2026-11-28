use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::fields::payload::records::{DecimalValue, ErrorInformation};
use betterlite_logging::payload::deps::TagConfig;
use betterlite_logging::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use betterlite_logging::prometheus::budget_lookups::lookups::{
    BudgetLookupError, BudgetLookups, UserLookupOptions,
};
use betterlite_logging::prometheus::budget_lookups::records::{
    CachedEndUser, KeyRecord, OrgRecord, TeamRecord, UserRecord,
};
use betterlite_logging::prometheus::failure::RequestedModelCatalog;
use betterlite_logging::prometheus_exporter::{PrometheusCollectors, PrometheusExporterSettings};
use betterlite_logging::prometheus_helpers::bounded_series_tracker::SeriesLimits;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_logging::prometheus_rate_limits::{RemainingModelGroupUsage, RemainingUsageError};
use betterlite_models::decisions::Json;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use betterlite_prometheus::collector_registry::CollectorRegistry;
use betterlite_prometheus::collector_registry::metric::sample::Sample;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::prometheus_hooks::budget::BudgetLookupPort;
use crate::prometheus_hooks::hooks::{PrometheusHookPorts, PrometheusHooks};
use crate::prometheus_hooks::settings::PrometheusHookSettings;
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
pub(crate) const DEPLOYMENT_TOTAL: &str = "litellm_deployment_total_requests";
pub(crate) const DEPLOYMENT_STATE: &str = "litellm_deployment_state";
pub(crate) const DEPLOYMENT_PER_OUTPUT_TOKEN: &str = "litellm_deployment_latency_per_output_token";
pub(crate) const ZERO_COST: &str = "litellm_zero_cost_requests_total";
pub(crate) const TOTAL_TOKENS: &str = "litellm_total_tokens_metric";
pub(crate) const INPUT_TOKENS: &str = "litellm_input_tokens_metric";
pub(crate) const OUTPUT_TOKENS: &str = "litellm_output_tokens_metric";
pub(crate) const INPUT_CACHED_TOKENS: &str = "litellm_input_cached_tokens_metric";
pub(crate) const INPUT_CACHE_CREATION_TOKENS: &str = "litellm_input_cache_creation_tokens_metric";
pub(crate) const INPUT_AUDIO_TOKENS: &str = "litellm_input_audio_tokens_metric";
pub(crate) const OUTPUT_REASONING_TOKENS: &str = "litellm_output_reasoning_tokens_metric";
pub(crate) const OUTPUT_AUDIO_TOKENS: &str = "litellm_output_audio_tokens_metric";
pub(crate) const VIDEO_DURATION: &str = "litellm_video_duration_seconds_metric";
pub(crate) const IMAGES_GENERATED: &str = "litellm_images_generated_metric";
pub(crate) const CACHE_HITS: &str = "litellm_cache_hits_metric";
pub(crate) const CACHED_TOKENS: &str = "litellm_cached_tokens_metric";
pub(crate) const PROVIDER_CACHE_READ: &str = "litellm_provider_cache_read_input_tokens_metric";
pub(crate) const PROVIDER_CACHE_CREATION: &str =
    "litellm_provider_cache_creation_input_tokens_metric";
pub(crate) const MCP_CALLS: &str = "litellm_mcp_tool_calls_total";
pub(crate) const MCP_SPEND: &str = "litellm_mcp_tool_call_spend_metric";
pub(crate) const API_KEY_ALLOWED: &str = "litellm_api_key_rate_limit_allowed_metric";
pub(crate) const API_KEY_USED: &str = "litellm_api_key_rate_limit_used_metric";
pub(crate) const TEAM_ALLOWED: &str = "litellm_team_rate_limit_allowed_metric";
pub(crate) const TEAM_USED: &str = "litellm_team_rate_limit_used_metric";
pub(crate) const VIRTUAL_KEY_TOKENS: &str = "litellm_remaining_api_key_tokens_for_model";
pub(crate) const REMAINING_REQUESTS: &str = "litellm_remaining_requests_metric";
pub(crate) const REMAINING_TOKENS: &str = "litellm_remaining_tokens_metric";
pub(crate) const TTFT: &str = "litellm_llm_api_time_to_first_token_metric";
pub(crate) const TOTAL_LATENCY: &str = "litellm_request_total_latency_metric";
pub(crate) const QUEUE_TIME: &str = "litellm_request_queue_time_seconds";
pub(crate) const OVERHEAD_LATENCY: &str = "litellm_overhead_latency_metric";

// the cache and database reads the fan-out makes, recorded so a test can see which ids reached them
pub(crate) struct RecordingLookups {
    org_record: Mutex<Option<OrgRecord>>,
    org_errors: AtomicBool,
    keys: Mutex<Vec<String>>,
    orgs: Mutex<Vec<String>>,
}

impl RecordingLookups {
    fn new() -> Self {
        Self {
            org_record: Mutex::new(None),
            org_errors: AtomicBool::new(false),
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

    pub(crate) fn fail_org_info(&self) {
        self.org_errors.store(true, Ordering::SeqCst);
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
        if self.org_errors.load(Ordering::SeqCst) {
            return Err(BudgetLookupError {
                message: "the org store is down".to_owned(),
            });
        }
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

// the requested-model bound: every name in the set is vouched for, and anything else falls to "other"
pub(crate) struct RecordingCatalog {
    recognized: BTreeSet<String>,
}

impl RecordingCatalog {
    pub(crate) fn new(names: &[&str]) -> Self {
        Self {
            recognized: names.iter().map(|name| (*name).to_owned()).collect(),
        }
    }
}

impl RequestedModelCatalog for RecordingCatalog {
    fn is_recognized_model(&self, model: &str) -> bool {
        self.recognized.contains(model)
    }

    fn is_team_public_model(&self, model: &str) -> bool {
        self.recognized.contains(model)
    }

    fn matches_pattern_route(&self, model: &str) -> bool {
        self.recognized.contains(model)
    }
}

// the router's remaining usage for one model group, recorded so a test can see the group it asked about
pub(crate) struct RecordingRouterUsage {
    answer: BTreeMap<String, i64>,
    groups: Mutex<Vec<String>>,
}

impl RecordingRouterUsage {
    pub(crate) fn new(requests: i64, tokens: i64) -> Self {
        let answer = BTreeMap::from([
            ("x-ratelimit-remaining-requests".to_owned(), requests),
            ("x-ratelimit-remaining-tokens".to_owned(), tokens),
        ]);
        Self {
            answer,
            groups: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn groups(&self) -> Vec<String> {
        self.groups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl RemainingModelGroupUsage for RecordingRouterUsage {
    async fn get_remaining_model_group_usage(
        &self,
        model_group: &str,
    ) -> Result<BTreeMap<String, i64>, RemainingUsageError> {
        self.groups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model_group.to_owned());
        Ok(self.answer.clone())
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
    fixture_with_settings(PrometheusHookSettings {
        labels: PrometheusLabelSettings::with_defaults(),
        // the tracker only caps a metric whose end_user label carries a value, so the fixture lets the label through
        end_user: EndUserCostTrackingSettings {
            disable_end_user_cost_tracking: None,
            enable_prometheus_only: Some(true),
        },
        tags: TagConfig::default(),
        limits,
    })
}

pub(crate) fn labelled_settings() -> PrometheusHookSettings {
    PrometheusHookSettings {
        labels: PrometheusLabelSettings {
            emit_stream_label: true,
            emit_rate_limit_labels: true,
            custom_tags: vec!["prod".to_owned()],
            ..PrometheusLabelSettings::with_defaults()
        },
        end_user: EndUserCostTrackingSettings {
            disable_end_user_cost_tracking: None,
            enable_prometheus_only: Some(true),
        },
        tags: TagConfig::default(),
        limits: SeriesLimits::default(),
    }
}

pub(crate) fn fixture_with_settings(settings: PrometheusHookSettings) -> Fixture {
    fixture_with_ports(settings, PrometheusHookPorts::default())
}

pub(crate) fn fixture_with_ports(
    settings: PrometheusHookSettings,
    ports: PrometheusHookPorts,
) -> Fixture {
    build_fixture(settings, ports, PrometheusExporterSettings::default())
}

// both switches must be on for the label: the collector computes the bucket and the schema registers the label
pub(crate) fn fixture_with_input_sequence_length() -> Fixture {
    let mut labels = PrometheusLabelSettings::with_defaults();
    labels.budget_and_length.emit_input_sequence_length = true;
    let settings = PrometheusHookSettings {
        labels,
        end_user: EndUserCostTrackingSettings::default(),
        tags: TagConfig::default(),
        limits: SeriesLimits::default(),
    };
    let exporter = PrometheusExporterSettings {
        prometheus_metrics_config: Vec::new(),
        prometheus_exclude_metrics: Vec::new(),
        prometheus_exclude_labels: Vec::new(),
        prometheus_latency_buckets: None,
        prometheus_emit_input_sequence_length_label: true,
    };
    build_fixture(settings, PrometheusHookPorts::default(), exporter)
}

fn build_fixture(
    settings: PrometheusHookSettings,
    ports: PrometheusHookPorts,
    exporter: PrometheusExporterSettings,
) -> Fixture {
    let registry = CollectorRegistry::default();
    let collectors = Arc::new(
        PrometheusCollectors::register(&exporter, &settings.labels, &registry)
            .expect("the collectors register"),
    );
    let lookups = Arc::new(RecordingLookups::new());
    let port_lookups = Arc::clone(&lookups);
    let port = Arc::new(BudgetLookupPort::new(
        port_lookups,
        Arc::clone(&collectors),
        settings.labels.clone(),
        settings.limits,
        settings.end_user,
    ));
    let budget =
        RemainingBudgetFanOut::new(Arc::clone(&port), Duration::from_secs(1), &port.gauges);
    let hooks = PrometheusHooks::new(collectors, settings, ports, budget, port);
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

// the emitted series keeps its labels in the metric's registered order, so a name finds its value
pub(crate) fn series_label(registry: &CollectorRegistry, metric: &str, name: &str) -> String {
    let collector = registry.get(metric).expect("the metric is registered");
    let index = collector
        .description
        .label_names
        .iter()
        .position(|candidate| candidate == name)
        .expect("the label is registered");
    let series = collector.series_in_use_order();
    let (labels, _sample) = series.first().expect("the metric has a series");
    labels
        .get(index)
        .expect("the label value is emitted")
        .clone()
}

pub(crate) fn number(literal: &str) -> Json {
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

// a payload that trips every one of the success event's emitters, from the token details to the MCP metadata
pub(crate) fn rich_payload(response_cost: Decimal) -> StandardLoggingPayload {
    let mut record = payload(response_cost);
    record.model_group = Some("gpt-4-group".to_owned());
    record.model_id = Some("deployment-1".to_owned());
    record.api_base = "https://api.example/v1".to_owned();
    record.custom_llm_provider = Some("openai".to_owned());
    record.stream = Some(true);
    record.cache_hit = Some(true);
    record.total_tokens = 12;
    record.prompt_tokens = 8;
    record.completion_tokens = 4;
    record.start_time = dec("100");
    record.end_time = dec("103");
    record.completion_start_time = dec("101");
    record.request_tags = vec![Json::Text("prod".to_owned())];
    record.metadata.user_api_key_user_id = Some("user-1".to_owned());
    record.metadata.user_api_key_hash = Some("sk-live".to_owned());
    record.metadata.user_api_key_alias = Some("alias-1".to_owned());
    record.metadata.user_api_key_team_id = Some("team-1".to_owned());
    record.metadata.user_api_key_team_alias = Some("team-alias".to_owned());
    record.metadata.user_api_key_user_email = Some("user@example.com".to_owned());
    record.metadata.user_api_key_org_id = Some("org-1".to_owned());
    record.metadata.user_api_key_org_alias = Some("org-alias".to_owned());
    record.metadata.user_api_key_end_user_id = Some("end-user-1".to_owned());
    record.metadata.user_api_key_request_route = Some("/v1/chat/completions".to_owned());
    record.metadata.requester_ip_address = Some("10.0.0.7".to_owned());
    record.metadata.user_agent = Some("test-agent".to_owned());
    record.metadata.usage_object = Some(usage_object());
    record.metadata.mcp_tool_call_metadata = Some(mcp_metadata());
    record.zero_cost_diagnostic = Some(zero_cost_fields());
    record.hidden_params.additional_headers = Some(rate_limit_headers());
    record.hidden_params.litellm_overhead_time_ms = Some(DecimalValue(dec("40")));
    record
}

fn usage_object() -> Json {
    Json::Object(IndexMap::from([
        ("prompt_tokens".to_owned(), number("8")),
        ("completion_tokens".to_owned(), number("4")),
        ("service_tier".to_owned(), Json::Text("priority".to_owned())),
        ("duration_seconds".to_owned(), number("2.5")),
        ("output_image_count".to_owned(), number("2")),
        ("cache_read_input_tokens".to_owned(), number("3")),
        ("cache_creation_input_tokens".to_owned(), number("1")),
        (
            "prompt_tokens_details".to_owned(),
            Json::Object(IndexMap::from([
                ("cached_tokens".to_owned(), number("3")),
                ("audio_tokens".to_owned(), number("1")),
                ("cache_write_tokens".to_owned(), number("1")),
            ])),
        ),
        (
            "completion_tokens_details".to_owned(),
            Json::Object(IndexMap::from([
                ("reasoning_tokens".to_owned(), number("2")),
                ("audio_tokens".to_owned(), number("1")),
            ])),
        ),
    ]))
}

fn mcp_metadata() -> Json {
    let entries = [("name", "tool-1"), ("mcp_server_name", "server-1")];
    Json::Object(
        entries
            .into_iter()
            .map(|(name, value)| (name.to_owned(), Json::Text(value.to_owned())))
            .collect(),
    )
}

fn zero_cost_fields() -> Json {
    let mut fields = IndexMap::new();
    fields.insert(
        "reason".to_owned(),
        Json::Text("missing_pricing_key".to_owned()),
    );
    fields.insert("pricing_model".to_owned(), Json::Text("gpt-4o".to_owned()));
    fields.insert(
        "missing_pricing_keys".to_owned(),
        Json::List(vec![Json::Text("input_cost_per_token".to_owned())]),
    );
    Json::Object(fields)
}

fn rate_limit_headers() -> IndexMap<String, Json> {
    IndexMap::from([
        (
            "x-ratelimit-api_key-limit-requests".to_owned(),
            number("10"),
        ),
        (
            "x-ratelimit-api_key-remaining-requests".to_owned(),
            number("6"),
        ),
        ("x-ratelimit-team-limit-tokens".to_owned(), number("100")),
        ("x-ratelimit-team-remaining-tokens".to_owned(), number("70")),
    ])
}
