#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::str::FromStr as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::alert_settings::types::{
    AlertType, AlertTypeConfig, DEFAULT_ALERT_TYPES,
};
use betterlite_config::sections::alert_settings::webhooks::WebhookUrl;
use betterlite_config::sections::langfuse_settings;
use chrono::{FixedOffset, TimeZone as _};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::SecretString;
use strum::{EnumCount as _, IntoEnumIterator as _};
use tokio::time::Instant;

use crate::alert_settings::cache_keys::SlackAlertingCacheKeys;
use crate::alert_settings::records::{AlertQueueItem, AlertText, DigestEntry, HangingRequestData};
use crate::alert_settings::trace::{LangfuseTraceContext, LangfuseTraceSource, langfuse_trace_url};
use crate::alert_settings::{
    HANGING_ALERT_BUFFER_SECONDS, LITELLM_LOGO_URL, LITELLM_SUPPORT_CONTACT,
    MAX_OLDEST_HANGING_REQUESTS, SPEND_THRESHOLD_5_PERCENT, SPEND_THRESHOLD_15_PERCENT,
    all_possible_alert_types,
};

fn lookup(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn langfuse_settings(variables: &[(&str, &str)]) -> langfuse_settings::Settings {
    let config = ConfigLoader {
        lookup: lookup(variables),
    }
    .load("")
    .expect("the langfuse settings load");
    config.sections.langfuse_settings
}

fn time() -> chrono::DateTime<FixedOffset> {
    FixedOffset::east_opt(0)
        .expect("UTC is a valid offset")
        .with_ymd_and_hms(2026, 1, 1, 12, 0, 0)
        .single()
        .expect("the test time is unambiguous")
}

fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the test decimal parses")
}

struct FakeTraceSource {
    ids: Mutex<VecDeque<Option<String>>>,
    calls: AtomicUsize,
}

impl FakeTraceSource {
    fn with_ids(ids: &[Option<&str>]) -> Self {
        let queued = ids
            .iter()
            .map(|id| id.map(str::to_owned))
            .collect::<VecDeque<_>>();
        Self {
            ids: Mutex::new(queued),
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl LangfuseTraceSource for FakeTraceSource {
    fn trace_id(&self) -> Option<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut ids = self.ids.lock().expect("the fake queue lock is free");
        ids.pop_front().flatten()
    }
}

#[tokio::test(start_paused = true)]
async fn trace_lookup_without_a_langfuse_callback_returns_none() {
    let settings = langfuse_settings(&[]);
    let source = FakeTraceSource::with_ids(&[Some("abc")]);
    let context = LangfuseTraceContext {
        langfuse_registered: false,
        dynamic_host: None,
        instance_host: None,
    };
    let start = Instant::now();
    let url = langfuse_trace_url(&settings, context, Some(&source)).await;
    assert_eq!(url, None);
    assert_eq!(source.calls(), 0);
    assert_eq!(start.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn trace_lookup_without_a_logging_object_returns_none() {
    let settings = langfuse_settings(&[]);
    let context = LangfuseTraceContext {
        langfuse_registered: true,
        dynamic_host: None,
        instance_host: None,
    };
    let start = Instant::now();
    let url = langfuse_trace_url(&settings, context, None).await;
    assert_eq!(url, None);
    assert_eq!(start.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn trace_lookup_retries_three_times_before_giving_up() {
    let settings = langfuse_settings(&[]);
    let source = FakeTraceSource::with_ids(&[None, None, None]);
    let context = LangfuseTraceContext {
        langfuse_registered: true,
        dynamic_host: None,
        instance_host: None,
    };
    let start = Instant::now();
    let url = langfuse_trace_url(&settings, context, Some(&source)).await;
    assert_eq!(url, None);
    assert_eq!(source.calls(), 3);
    assert_eq!(start.elapsed(), Duration::from_secs(9));
}

#[tokio::test(start_paused = true)]
async fn trace_lookup_returns_the_url_when_the_trace_id_appears() {
    let settings = langfuse_settings(&[]);
    let source = FakeTraceSource::with_ids(&[None, Some("abc")]);
    let context = LangfuseTraceContext {
        langfuse_registered: true,
        dynamic_host: None,
        instance_host: None,
    };
    let start = Instant::now();
    let url = langfuse_trace_url(&settings, context, Some(&source)).await;
    assert_eq!(url.as_deref(), Some("https://cloud.langfuse.com/trace/abc"));
    assert_eq!(source.calls(), 2);
    assert_eq!(start.elapsed(), Duration::from_secs(3));
}

#[tokio::test(start_paused = true)]
async fn trace_lookup_uses_the_dynamic_host_over_the_instance_host() {
    let settings = langfuse_settings(&[]);
    let source = FakeTraceSource::with_ids(&[Some("abc")]);
    let context = LangfuseTraceContext {
        langfuse_registered: true,
        dynamic_host: Some("https://dynamic.example"),
        instance_host: Some("https://instance.example"),
    };
    let url = langfuse_trace_url(&settings, context, Some(&source)).await;
    assert_eq!(url.as_deref(), Some("https://dynamic.example/trace/abc"));
}

#[tokio::test(start_paused = true)]
async fn trace_lookup_falls_back_to_the_instance_host() {
    let settings = langfuse_settings(&[]);
    let source = FakeTraceSource::with_ids(&[Some("abc")]);
    let context = LangfuseTraceContext {
        langfuse_registered: true,
        dynamic_host: None,
        instance_host: Some("https://instance.example"),
    };
    let url = langfuse_trace_url(&settings, context, Some(&source)).await;
    assert_eq!(url.as_deref(), Some("https://instance.example/trace/abc"));
}

#[tokio::test(start_paused = true)]
async fn trace_lookup_ignores_an_empty_dynamic_host() {
    let settings = langfuse_settings(&[]);
    let source = FakeTraceSource::with_ids(&[Some("abc")]);
    let context = LangfuseTraceContext {
        langfuse_registered: true,
        dynamic_host: Some(""),
        instance_host: Some("https://instance.example"),
    };
    let url = langfuse_trace_url(&settings, context, Some(&source)).await;
    assert_eq!(url.as_deref(), Some("https://instance.example/trace/abc"));
}

#[tokio::test(start_paused = true)]
async fn trace_lookup_resolves_a_langfuse_host_from_the_environment() {
    let settings = langfuse_settings(&[("LANGFUSE_HOST", "langfuse.internal:3000")]);
    let source = FakeTraceSource::with_ids(&[Some("abc")]);
    let context = LangfuseTraceContext {
        langfuse_registered: true,
        dynamic_host: None,
        instance_host: None,
    };
    let url = langfuse_trace_url(&settings, context, Some(&source)).await;
    assert_eq!(
        url.as_deref(),
        Some("http://langfuse.internal:3000/trace/abc")
    );
}

#[tokio::test(start_paused = true)]
async fn trace_lookup_defaults_to_the_langfuse_cloud_host() {
    let settings = langfuse_settings(&[]);
    let source = FakeTraceSource::with_ids(&[Some("abc")]);
    let context = LangfuseTraceContext {
        langfuse_registered: true,
        dynamic_host: None,
        instance_host: None,
    };
    let url = langfuse_trace_url(&settings, context, Some(&source)).await;
    assert_eq!(url.as_deref(), Some("https://cloud.langfuse.com/trace/abc"));
}

#[test]
fn all_possible_alert_types_lists_every_enum_member() {
    let listed = all_possible_alert_types();
    assert_eq!(AlertType::COUNT, 27);
    assert_eq!(listed.len(), AlertType::COUNT);
    assert!(AlertType::iter().all(|alert_type| listed.contains(&alert_type)));
}

#[test]
fn default_alert_types_match_litellm() {
    let expected = [
        AlertType::LlmExceptions,
        AlertType::LlmTooSlow,
        AlertType::LlmRequestsHanging,
        AlertType::BudgetAlerts,
        AlertType::SpendReports,
        AlertType::FailedTrackingSpend,
        AlertType::UserSpendThresholds,
        AlertType::DbExceptions,
        AlertType::DailyReports,
        AlertType::CooldownDeployment,
        AlertType::NewModelAdded,
        AlertType::ModelDeprecationWarnings,
        AlertType::OutageAlerts,
        AlertType::RegionOutageAlerts,
        AlertType::FallbackReports,
        AlertType::McpToolDescriptionBlocked,
        AlertType::McpPinnedToolsChanged,
    ];
    assert_eq!(DEFAULT_ALERT_TYPES, expected);
    assert!(!DEFAULT_ALERT_TYPES.contains(&AlertType::UserSpendAnomalies));
}

#[rstest]
#[case::llm_exceptions(AlertType::LlmExceptions, "llm_exceptions")]
#[case::llm_too_slow(AlertType::LlmTooSlow, "llm_too_slow")]
#[case::llm_requests_hanging(AlertType::LlmRequestsHanging, "llm_requests_hanging")]
#[case::budget_alerts(AlertType::BudgetAlerts, "budget_alerts")]
#[case::spend_reports(AlertType::SpendReports, "spend_reports")]
#[case::failed_tracking_spend(AlertType::FailedTrackingSpend, "failed_tracking_spend")]
#[case::user_spend_thresholds(AlertType::UserSpendThresholds, "user_spend_thresholds")]
#[case::user_spend_anomalies(AlertType::UserSpendAnomalies, "user_spend_anomalies")]
#[case::db_exceptions(AlertType::DbExceptions, "db_exceptions")]
#[case::daily_reports(AlertType::DailyReports, "daily_reports")]
#[case::cooldown_deployment(AlertType::CooldownDeployment, "cooldown_deployment")]
#[case::new_model_added(AlertType::NewModelAdded, "new_model_added")]
#[case::model_deprecation_warnings(
    AlertType::ModelDeprecationWarnings,
    "model_deprecation_warnings"
)]
#[case::outage_alerts(AlertType::OutageAlerts, "outage_alerts")]
#[case::region_outage_alerts(AlertType::RegionOutageAlerts, "region_outage_alerts")]
#[case::fallback_reports(AlertType::FallbackReports, "fallback_reports")]
#[case::new_virtual_key_created(AlertType::NewVirtualKeyCreated, "new_virtual_key_created")]
#[case::virtual_key_updated(AlertType::VirtualKeyUpdated, "virtual_key_updated")]
#[case::virtual_key_deleted(AlertType::VirtualKeyDeleted, "virtual_key_deleted")]
#[case::new_team_created(AlertType::NewTeamCreated, "new_team_created")]
#[case::team_updated(AlertType::TeamUpdated, "team_updated")]
#[case::team_deleted(AlertType::TeamDeleted, "team_deleted")]
#[case::new_internal_user_created(AlertType::NewInternalUserCreated, "new_internal_user_created")]
#[case::internal_user_updated(AlertType::InternalUserUpdated, "internal_user_updated")]
#[case::internal_user_deleted(AlertType::InternalUserDeleted, "internal_user_deleted")]
#[case::mcp_tool_description_blocked(
    AlertType::McpToolDescriptionBlocked,
    "mcp_tool_description_blocked"
)]
#[case::mcp_pinned_tools_changed(AlertType::McpPinnedToolsChanged, "mcp_pinned_tools_changed")]
fn alert_type_names_match_litellm(#[case] alert_type: AlertType, #[case] name: &str) {
    let encoded = serde_json::to_string(&alert_type).expect("an alert type serializes");
    assert_eq!(encoded, format!("\"{name}\""));
}

#[test]
fn alerting_constants_match_litellm() {
    assert_eq!(SPEND_THRESHOLD_5_PERCENT, decimal("0.05"));
    assert_eq!(SPEND_THRESHOLD_15_PERCENT, decimal("0.15"));
    assert_eq!(MAX_OLDEST_HANGING_REQUESTS, 20);
    assert_eq!(HANGING_ALERT_BUFFER_SECONDS, 60);
    assert_eq!(
        LITELLM_LOGO_URL,
        "https://litellm-listing.s3.amazonaws.com/litellm_logo.png"
    );
    assert_eq!(LITELLM_SUPPORT_CONTACT, "support@berri.ai");
}

#[test]
fn cache_keys_match_litellm() {
    assert_eq!(
        SlackAlertingCacheKeys::FailedRequestsDailyMetrics.to_string(),
        "failed_requests_daily_metrics"
    );
    assert_eq!(
        SlackAlertingCacheKeys::LatencyDailyMetrics.to_string(),
        "latency_daily_metrics"
    );
    assert_eq!(
        SlackAlertingCacheKeys::DailyMetricsReportSent.to_string(),
        "daily_metrics_report_sent"
    );
    assert_eq!(
        SlackAlertingCacheKeys::ModelDeprecationAlertSent.to_string(),
        "model_deprecation_alert_sent"
    );
}

#[test]
fn alert_type_config_defaults_to_digest_off_for_a_day() {
    let config = AlertTypeConfig::default();
    assert!(!config.digest);
    assert_eq!(config.digest_interval, 86_400);
}

#[test]
fn alert_queue_item_debug_hides_the_webhook_url() {
    let item = AlertQueueItem {
        url: SecretString::from("https://hooks.example/SECRET"),
        headers: BTreeMap::new(),
        payload: AlertText {
            text: "boom".to_owned(),
        },
        alert_type: AlertType::BudgetAlerts,
        format: None,
    };
    let debug = format!("{item:?}");
    assert!(!debug.contains("SECRET"), "{debug}");
    assert!(debug.contains("REDACTED"), "{debug}");
}

#[test]
fn hanging_request_data_new_applies_litellm_defaults() {
    let data = HangingRequestData::new("call-1".to_owned(), "gpt-4o".to_owned(), 1234.5);
    assert_eq!(data.request_id, "call-1");
    assert_eq!(data.model, "gpt-4o");
    assert_eq!(data.api_base, None);
    assert_eq!(data.key_alias, None);
    assert_eq!(data.team_alias, None);
    assert_eq!(data.alerting_metadata, None);
    assert_eq!(data.created_at.to_bits(), 1_234.5_f64.to_bits());
    assert!(!data.alerted);
}

#[test]
fn digest_entry_debug_hides_the_webhook_url() {
    let entry = DigestEntry {
        alert_type: "spend_reports".to_owned(),
        request_model: "gpt-4o".to_owned(),
        api_base: "https://api.example".to_owned(),
        first_message: "first".to_owned(),
        level: "High".to_owned(),
        count: 1,
        start_time: time(),
        last_time: time(),
        webhook_url: WebhookUrl::Multiple(vec![SecretString::from("https://hooks.example/SECRET")]),
    };
    let debug = format!("{entry:?}");
    assert!(!debug.contains("SECRET"), "{debug}");
    assert!(debug.contains("REDACTED"), "{debug}");
}
