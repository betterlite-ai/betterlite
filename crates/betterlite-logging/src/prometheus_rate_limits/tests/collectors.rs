#![cfg(test)]

use betterlite_prometheus::collector_factory::collector::{
    Collector as _, MetricLike, MetricValue,
};
use betterlite_prometheus::collector_registry::CollectorRegistry;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{
    Fixture, ScriptedUsage, json_object, number, payload, with_headers, with_model_group,
};
use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus_exporter::{Handle, PrometheusCollectors, PrometheusExporterSettings};
use crate::prometheus_label_schema::{PrometheusLabelSettings, get_labels};
use crate::prometheus_rate_limits::deployment::{
    DeploymentLimitInputs, set_deployment_tpm_rpm_limit_metrics,
};
use crate::prometheus_rate_limits::gauge::RateLimitGauges as _;
use crate::prometheus_rate_limits::key_and_team::set_key_and_team_rate_limit_metrics;
use crate::prometheus_rate_limits::metric::RateLimitMetric;
use crate::prometheus_rate_limits::router_remaining::{
    RouterRemainingInputs, async_set_router_remaining_metrics,
};
use crate::prometheus_rate_limits::virtual_key::{
    VirtualKeyRateLimitInputs, set_virtual_key_rate_limit_metrics,
};

const API_KEY_ALLOWED: &str = "litellm_api_key_rate_limit_allowed_metric";
const API_KEY_USED: &str = "litellm_api_key_rate_limit_used_metric";
const KEY_REQUESTS: &str = "litellm_remaining_api_key_requests_for_model";
const KEY_TOKENS: &str = "litellm_remaining_api_key_tokens_for_model";
const DEPLOYMENT_TPM: &str = "litellm_deployment_tpm_limit";
const DEPLOYMENT_RPM: &str = "litellm_deployment_rpm_limit";
const ROUTER_TOKENS: &str = "litellm_remaining_tokens_metric";
const ROUTER_REQUESTS: &str = "litellm_remaining_requests_metric";

fn registered() -> (CollectorRegistry, PrometheusCollectors) {
    let registry = CollectorRegistry::default();
    let collectors = PrometheusCollectors::register(
        &PrometheusExporterSettings::default(),
        &PrometheusLabelSettings::with_defaults(),
        &registry,
    )
    .expect("the collectors register");
    (registry, collectors)
}

fn total(
    registry: &CollectorRegistry,
    metric: &str,
    series: &[Option<String>],
) -> Option<MetricValue> {
    registry
        .get(metric)?
        .series(series)
        .map(|sample| sample.total)
}

fn registered_name(handle: &Handle) -> &str {
    match handle {
        MetricLike::Real(metric) => &metric.description.name,
        MetricLike::Excluded(excluded) => &excluded.metric.description.name,
        MetricLike::NoOp(_) => panic!("the collector is a no-op"),
    }
}

#[rstest]
#[case::api_key_allowed(
    RateLimitMetric::ApiKeyRateLimitAllowed,
    "litellm_api_key_rate_limit_allowed_metric"
)]
#[case::api_key_used(
    RateLimitMetric::ApiKeyRateLimitUsed,
    "litellm_api_key_rate_limit_used_metric"
)]
#[case::team_allowed(
    RateLimitMetric::TeamRateLimitAllowed,
    "litellm_team_rate_limit_allowed_metric"
)]
#[case::team_used(
    RateLimitMetric::TeamRateLimitUsed,
    "litellm_team_rate_limit_used_metric"
)]
#[case::key_requests(
    RateLimitMetric::RemainingApiKeyRequests,
    "litellm_remaining_api_key_requests_for_model"
)]
#[case::key_tokens(
    RateLimitMetric::RemainingApiKeyTokens,
    "litellm_remaining_api_key_tokens_for_model"
)]
#[case::tpm(RateLimitMetric::DeploymentTpmLimit, "litellm_deployment_tpm_limit")]
#[case::rpm(RateLimitMetric::DeploymentRpmLimit, "litellm_deployment_rpm_limit")]
#[case::remaining_requests(
    RateLimitMetric::RemainingRequests,
    "litellm_remaining_requests_metric"
)]
#[case::remaining_tokens(RateLimitMetric::RemainingTokens, "litellm_remaining_tokens_metric")]
fn rate_limit_metrics_carry_the_litellm_names(
    #[case] metric: RateLimitMetric,
    #[case] expected: &str,
) {
    assert_eq!(metric.name(), expected);
}

#[rstest]
#[case::api_key_allowed(
    RateLimitMetric::ApiKeyRateLimitAllowed,
    "litellm_api_key_rate_limit_allowed_metric"
)]
#[case::api_key_used(
    RateLimitMetric::ApiKeyRateLimitUsed,
    "litellm_api_key_rate_limit_used_metric"
)]
#[case::team_allowed(
    RateLimitMetric::TeamRateLimitAllowed,
    "litellm_team_rate_limit_allowed_metric"
)]
#[case::team_used(
    RateLimitMetric::TeamRateLimitUsed,
    "litellm_team_rate_limit_used_metric"
)]
#[case::key_requests(
    RateLimitMetric::RemainingApiKeyRequests,
    "litellm_remaining_api_key_requests_for_model"
)]
#[case::key_tokens(
    RateLimitMetric::RemainingApiKeyTokens,
    "litellm_remaining_api_key_tokens_for_model"
)]
#[case::tpm(RateLimitMetric::DeploymentTpmLimit, "litellm_deployment_tpm_limit")]
#[case::rpm(RateLimitMetric::DeploymentRpmLimit, "litellm_deployment_rpm_limit")]
#[case::remaining_requests(
    RateLimitMetric::RemainingRequests,
    "litellm_remaining_requests_metric"
)]
#[case::remaining_tokens(RateLimitMetric::RemainingTokens, "litellm_remaining_tokens_metric")]
fn collectors_resolve_each_rate_limit_metric(
    #[case] metric: RateLimitMetric,
    #[case] expected: &str,
) {
    let (registry, collectors) = registered();
    assert_eq!(registered_name(&collectors.gauge(metric)), expected);
    assert!(registry.get(expected).is_some());
    assert_eq!(
        collectors.supported_labels(metric),
        get_labels(expected, &PrometheusLabelSettings::with_defaults())
            .expect("the metric has a label schema")
            .as_slice()
    );
}

#[test]
fn real_key_and_team_gauges_write_and_remove_series() {
    let (registry, collectors) = registered();
    let fixture = Fixture::new(&[("hashed_api_key", "key-1")]);
    let payload = with_headers(
        payload(),
        &[
            ("x-ratelimit-api_key-limit-requests", number("100")),
            ("x-ratelimit-api_key-remaining-requests", number("40")),
        ],
    );
    set_key_and_team_rate_limit_metrics(&collectors, &payload, fixture.inputs()).unwrap();
    let series = [Some("key-1".to_owned()), None, Some("requests".to_owned())];
    assert_eq!(
        total(&registry, API_KEY_ALLOWED, &series),
        Some(MetricValue::Finite(Decimal::from(100_i64)))
    );
    assert_eq!(
        total(&registry, API_KEY_USED, &series),
        Some(MetricValue::Finite(Decimal::from(60_i64)))
    );
    set_key_and_team_rate_limit_metrics(&collectors, &payload(), fixture.inputs()).unwrap();
    assert_eq!(total(&registry, API_KEY_ALLOWED, &series), None);
    assert_eq!(total(&registry, API_KEY_USED, &series), None);
}

#[test]
fn real_virtual_key_gauges_write_series() {
    let (registry, collectors) = registered();
    let settings = PrometheusLabelSettings::with_defaults();
    let metadata = json_object(&[("litellm-key-remaining-requests-gpt-4o", number("11"))]);
    let inputs = VirtualKeyRateLimitInputs {
        user_api_key: Some("key-1"),
        user_api_key_alias: Some("alias-1"),
        model_group: Some("gpt-4o"),
        model_id: Some("model-1"),
        metadata: &metadata,
        payload: None,
        settings: &settings,
        end_user_settings: EndUserCostTrackingSettings::default(),
    };
    set_virtual_key_rate_limit_metrics(&collectors, inputs).unwrap();
    let series = [
        Some("key-1".to_owned()),
        Some("alias-1".to_owned()),
        Some("gpt-4o".to_owned()),
        Some("model-1".to_owned()),
    ];
    assert_eq!(
        total(&registry, KEY_REQUESTS, &series),
        Some(MetricValue::Finite(Decimal::from(11_i64)))
    );
    assert_eq!(
        total(&registry, KEY_TOKENS, &series),
        Some(MetricValue::Finite(Decimal::from(i64::MAX)))
    );
}

#[test]
fn real_deployment_gauges_write_series() {
    let (registry, collectors) = registered();
    let settings = PrometheusLabelSettings::with_defaults();
    let model_info = json_object(&[("tpm", number("10")), ("rpm", number("20"))]);
    let litellm_params = json_object(&[]);
    let inputs = DeploymentLimitInputs {
        model_info: &model_info,
        litellm_params: &litellm_params,
        litellm_model_name: Some("openai/gpt-4o"),
        model_id: Some("model-1"),
        api_base: Some("https://api.openai.com/v1"),
        api_provider: Some("openai"),
        model_group: Some("gpt-4o"),
        settings: &settings,
        end_user_settings: EndUserCostTrackingSettings::default(),
    };
    set_deployment_tpm_rpm_limit_metrics(&collectors, inputs).unwrap();
    let series = [
        Some("gpt-4o".to_owned()),
        Some("openai/gpt-4o".to_owned()),
        Some("model-1".to_owned()),
        Some("https://api.openai.com/v1".to_owned()),
        Some("openai".to_owned()),
    ];
    assert_eq!(
        total(&registry, DEPLOYMENT_TPM, &series),
        Some(MetricValue::Finite(Decimal::from(10_i64)))
    );
    assert_eq!(
        total(&registry, DEPLOYMENT_RPM, &series),
        Some(MetricValue::Finite(Decimal::from(20_i64)))
    );
}

#[tokio::test]
async fn real_router_remaining_gauges_write_series() {
    let (registry, collectors) = registered();
    let fixture = Fixture::new(&[
        ("model_group", "gpt-4o"),
        ("api_provider", "openai"),
        ("api_base", ""),
        ("litellm_model_name", "openai/gpt-4o"),
        ("hashed_api_key", "key-1"),
        ("model_id", "model-1"),
    ]);
    let payload = with_model_group(payload(), "gpt-4o");
    let usage = ScriptedUsage::answering(&[("x-ratelimit-remaining-tokens", 900)]);
    let inputs = RouterRemainingInputs {
        payload: &payload,
        values: &fixture.values,
        context: Some(&fixture.context),
        settings: &fixture.settings,
        end_user_settings: EndUserCostTrackingSettings::default(),
        usage: Some(&usage),
    };
    async_set_router_remaining_metrics(&collectors, inputs).await;
    let series = [
        Some("gpt-4o".to_owned()),
        Some("openai".to_owned()),
        Some(String::new()),
        Some("openai/gpt-4o".to_owned()),
        Some("key-1".to_owned()),
        None,
        Some("model-1".to_owned()),
    ];
    assert_eq!(
        total(&registry, ROUTER_TOKENS, &series),
        Some(MetricValue::Finite(Decimal::from(900_i64)))
    );
    assert_eq!(total(&registry, ROUTER_REQUESTS, &series), None);
}
