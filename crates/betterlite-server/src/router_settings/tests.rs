#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::settings::{FieldSource, JsonValue};
use betterlite_config::types::Config;
use rstest::rstest;
use serde::Deserialize;
use tower::ServiceExt as _;

use crate::routes::router;
use crate::routes::tests::state_for;

use super::catalog::field_default;
use super::response::{RouterSettingsResponse, fields_response, settings_response};
use super::view::{current_values, settings_store};

fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

fn load(document: &str) -> Config {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    loader.load(document).unwrap()
}

fn response(document: &str) -> RouterSettingsResponse {
    let config = load(document);
    let store = settings_store(&config);
    settings_response(current_values(&config), &store)
}

#[rstest]
#[case::strategy_default(
    "router_settings: {}",
    "routing_strategy",
    r#""simple-shuffle""#,
    FieldSource::Default
)]
#[case::strategy_configured(
    "router_settings: {routing_strategy: least-busy}",
    "routing_strategy",
    r#""least-busy""#,
    FieldSource::Config
)]
#[case::retries_default("router_settings: {}", "num_retries", "2", FieldSource::Default)]
#[case::retries_configured(
    "router_settings: {num_retries: 4}",
    "num_retries",
    "4",
    FieldSource::Config
)]
#[case::timeout_default("router_settings: {}", "timeout", "6000.0", FieldSource::Default)]
#[case::timeout_configured(
    "router_settings: {timeout: 30.5}",
    "timeout",
    "30.5",
    FieldSource::Config
)]
#[case::stream_timeout_unset("router_settings: {}", "stream_timeout", "null", FieldSource::Unset)]
#[case::stream_timeout_configured(
    "router_settings: {stream_timeout: 7.5}",
    "stream_timeout",
    "7.5",
    FieldSource::Config
)]
#[case::max_fallbacks_default("router_settings: {}", "max_fallbacks", "5", FieldSource::Default)]
#[case::fallbacks_unset("router_settings: {}", "fallbacks", "null", FieldSource::Default)]
#[case::fallbacks_configured(
    "router_settings: {fallbacks: [{gpt-4: [gpt-3.5-turbo]}]}",
    "fallbacks",
    r#"[{"gpt-4": ["gpt-3.5-turbo"]}]"#,
    FieldSource::Config
)]
#[case::context_window_default(
    "router_settings: {}",
    "context_window_fallbacks",
    "null",
    FieldSource::Default
)]
#[case::context_window_configured(
    "router_settings: {context_window_fallbacks: [{gpt-4: [gpt-3.5-turbo]}]}",
    "context_window_fallbacks",
    r#"[{"gpt-4": ["gpt-3.5-turbo"]}]"#,
    FieldSource::Config
)]
#[case::content_policy_default(
    "router_settings: {}",
    "content_policy_fallbacks",
    "null",
    FieldSource::Default
)]
#[case::content_policy_configured(
    "router_settings: {content_policy_fallbacks: [{gpt-4: [gpt-3.5-turbo]}]}",
    "content_policy_fallbacks",
    r#"[{"gpt-4": ["gpt-3.5-turbo"]}]"#,
    FieldSource::Config
)]
#[case::allowed_fails_default("router_settings: {}", "allowed_fails", "3", FieldSource::Default)]
#[case::allowed_fails_zero(
    "router_settings: {allowed_fails: 0}",
    "allowed_fails",
    "0",
    FieldSource::Config
)]
#[case::cooldown_default("router_settings: {}", "cooldown_time", "5", FieldSource::Default)]
#[case::cooldown_configured(
    "router_settings: {cooldown_time: 2.5}",
    "cooldown_time",
    "2.5",
    FieldSource::Config
)]
#[case::retry_after_default("router_settings: {}", "retry_after", "0", FieldSource::Default)]
#[case::retry_policy_unset("router_settings: {}", "retry_policy", "null", FieldSource::Unset)]
#[case::retry_policy_configured(
    "router_settings: {retry_policy: {BadRequestErrorRetries: 2}}",
    "retry_policy",
    r#"{"BadRequestErrorRetries": 2}"#,
    FieldSource::Config
)]
#[case::retry_policy_all_errors(
    "router_settings: {retry_policy: {BadRequestErrorRetries: 1, AuthenticationErrorRetries: 2, TimeoutErrorRetries: 3, RateLimitErrorRetries: 4, ContentPolicyViolationErrorRetries: 5, InternalServerErrorRetries: 6, ServiceUnavailableErrorRetries: 7, NotFoundErrorRetries: 8, DefaultRetries: 9}}",
    "retry_policy",
    r#"{"BadRequestErrorRetries": 1, "AuthenticationErrorRetries": 2, "TimeoutErrorRetries": 3, "RateLimitErrorRetries": 4, "ContentPolicyViolationErrorRetries": 5, "InternalServerErrorRetries": 6, "ServiceUnavailableErrorRetries": 7, "NotFoundErrorRetries": 8, "DefaultRetries": 9}"#,
    FieldSource::Config
)]
#[case::alias_unset("router_settings: {}", "model_group_alias", "{}", FieldSource::Default)]
#[case::alias_configured(
    "router_settings: {model_group_alias: {gpt-4: gpt-3.5-turbo}}",
    "model_group_alias",
    r#"{"gpt-4": "gpt-3.5-turbo"}"#,
    FieldSource::Config
)]
#[case::alias_hidden_configured(
    "router_settings: {model_group_alias: {gpt-4: {model: gpt-3.5-turbo, hidden: true}}}",
    "model_group_alias",
    r#"{"gpt-4": {"model": "gpt-3.5-turbo", "hidden": true}}"#,
    FieldSource::Config
)]
#[case::pre_call_checks_default(
    "router_settings: {}",
    "enable_pre_call_checks",
    "false",
    FieldSource::Default
)]
#[case::pre_call_checks_configured(
    "router_settings: {enable_pre_call_checks: true}",
    "enable_pre_call_checks",
    "true",
    FieldSource::Config
)]
#[case::max_parallel_unset(
    "router_settings: {}",
    "default_max_parallel_requests",
    "null",
    FieldSource::Unset
)]
#[case::max_parallel_configured(
    "router_settings: {default_max_parallel_requests: 8}",
    "default_max_parallel_requests",
    "8",
    FieldSource::Config
)]
#[case::verbose_default("router_settings: {}", "set_verbose", "false", FieldSource::Default)]
#[case::tag_filtering_default(
    "router_settings: {}",
    "tag_filtering_match_any",
    "true",
    FieldSource::Default
)]
#[case::cooldowns_unset("router_settings: {}", "disable_cooldowns", "null", FieldSource::Unset)]
#[case::cooldowns_enabled(
    "router_settings: {disable_cooldowns: true}",
    "disable_cooldowns",
    "true",
    FieldSource::Config
)]
#[case::groups_unset("router_settings: {}", "routing_groups", "[]", FieldSource::Default)]
#[case::strategy_args_unset(
    "router_settings: {}",
    "routing_strategy_args",
    "{}",
    FieldSource::Default
)]
#[case::computed_default_params(
    "router_settings: {timeout: 9.5}",
    "default_litellm_params",
    r#"{"max_retries": 0, "metadata": {"caching_groups": null}, "timeout": 9.5}"#,
    FieldSource::Default
)]
#[case::written_default_params(
    "router_settings: {default_litellm_params: {timeout: 9.5}}",
    "default_litellm_params",
    r#"{"timeout": 9.5}"#,
    FieldSource::Config
)]
fn router_settings_behaviors(
    #[case] document: &str,
    #[case] key: &str,
    #[case] expected_json: &str,
    #[case] expected_source: FieldSource,
) {
    let expected = json(expected_json);
    let answer = response(document);
    assert_eq!(answer.current_values.get(key), Some(&expected));
    assert_eq!(answer.source.get(key), Some(&expected_source));
    let field = answer
        .fields
        .iter()
        .find(|field| field.field_name == key)
        .unwrap();
    assert_eq!(field.field_value, expected);
}

#[rstest]
#[case::zero_retries(
    "router_settings: {num_retries: 0}",
    "num_retries",
    "0",
    FieldSource::Config
)]
#[case::zero_fails(
    "router_settings: {allowed_fails: 0}",
    "allowed_fails",
    "0",
    FieldSource::Config
)]
#[case::zero_max_fallbacks(
    "router_settings: {max_fallbacks: 0}",
    "max_fallbacks",
    "0",
    FieldSource::Default
)]
#[case::zero_cooldown(
    "router_settings: {cooldown_time: 0}",
    "cooldown_time",
    "0",
    FieldSource::Config
)]
#[case::zero_timeout("router_settings: {timeout: 0}", "timeout", "0", FieldSource::Config)]
#[case::zero_retry_after(
    "router_settings: {retry_after: 0}",
    "retry_after",
    "0",
    FieldSource::Default
)]
#[case::largest_allowed_fails(
    "router_settings: {allowed_fails: 4294967295}",
    "allowed_fails",
    "4294967295",
    FieldSource::Config
)]
#[case::largest_max_fallbacks(
    "router_settings: {max_fallbacks: 9223372036854775807}",
    "max_fallbacks",
    "9223372036854775807",
    FieldSource::Default
)]
#[case::empty_alias(
    "router_settings: {model_group_alias: {gpt-4: ''}}",
    "model_group_alias",
    r#"{"gpt-4": ""}"#,
    FieldSource::Config
)]
#[case::empty_groups(
    "router_settings: {routing_groups: []}",
    "routing_groups",
    "[]",
    FieldSource::Config
)]
#[case::fractional_retry_after(
    "router_settings: {retry_after: 0.5}",
    "retry_after",
    "0.5",
    FieldSource::Default
)]
#[case::configured_false_pre_call_checks(
    "router_settings: {enable_pre_call_checks: false}",
    "enable_pre_call_checks",
    "false",
    FieldSource::Default
)]
fn router_settings_boundaries(
    #[case] document: &str,
    #[case] key: &str,
    #[case] expected_json: &str,
    #[case] expected_source: FieldSource,
) {
    let answer = response(document);
    assert_eq!(answer.current_values.get(key), Some(&json(expected_json)));
    assert_eq!(answer.source.get(key), Some(&expected_source));
}

#[test]
fn fields_endpoint_reports_the_reviewed_catalog_without_values() {
    let answer = fields_response();
    let names: Vec<&str> = answer.fields.iter().map(|field| field.field_name).collect();
    assert_eq!(
        names,
        [
            "routing_strategy",
            "routing_strategy_args",
            "routing_groups",
            "num_retries",
            "timeout",
            "stream_timeout",
            "max_fallbacks",
            "fallbacks",
            "context_window_fallbacks",
            "content_policy_fallbacks",
            "allowed_fails",
            "cooldown_time",
            "retry_after",
            "retry_policy",
            "model_group_alias",
            "enable_pre_call_checks",
            "default_litellm_params",
            "set_verbose",
            "default_max_parallel_requests",
            "enable_tag_filtering",
            "tag_filtering_match_any",
            "disable_cooldowns",
        ]
    );
    assert!(
        answer
            .fields
            .iter()
            .all(|field| field.field_value.is_null())
    );
}

#[test]
fn only_the_routing_strategy_field_lists_options() {
    let answer = fields_response();
    let strategy = &answer.fields[0];
    assert_eq!(
        strategy.options.as_deref(),
        Some(
            [
                "simple-shuffle",
                "least-busy",
                "usage-based-routing",
                "latency-based-routing",
                "cost-based-routing",
                "usage-based-routing-v2",
                "lar1",
            ]
            .as_slice()
        )
    );
    assert!(
        answer
            .fields
            .iter()
            .skip(1)
            .all(|field| field.options.is_none())
    );
}

#[test]
fn the_fields_response_lists_the_routing_strategies_and_descriptions() {
    let answer = fields_response();
    assert_eq!(
        answer.routing_group_strategies,
        [
            "simple-shuffle",
            "least-busy",
            "usage-based-routing",
            "latency-based-routing",
            "cost-based-routing",
            "usage-based-routing-v2",
            "lar1",
            "priority",
        ]
    );
    assert_eq!(answer.routing_strategy_descriptions.len(), 7);
    assert_eq!(
        answer.routing_strategy_descriptions.get("priority"),
        Some(
            &"Routes group calls to the lowest-priority-number available model, with failover to higher numbers. Equal priorities share traffic. Direct member calls keep their existing policy."
        )
    );
}

#[test]
fn only_the_tag_filtering_field_carries_a_documentation_link() {
    let answer = fields_response();
    let tag = answer
        .fields
        .iter()
        .find(|field| field.field_name == "enable_tag_filtering")
        .unwrap();
    assert_eq!(
        tag.link,
        Some("https://docs.litellm.ai/docs/proxy/tag_routing")
    );
    assert!(
        answer
            .fields
            .iter()
            .filter(|field| field.field_name != "enable_tag_filtering")
            .all(|field| field.link.is_none())
    );
}

#[test]
fn field_defaults_are_read_from_the_reviewed_catalog() {
    let answer = fields_response();
    let defaults: BTreeMap<&str, JsonValue> = answer
        .fields
        .iter()
        .map(|field| (field.field_name, field.field_default.clone()))
        .collect();
    assert_eq!(
        defaults.get("routing_strategy"),
        Some(&json("simple-shuffle"))
    );
    assert_eq!(defaults.get("routing_groups"), Some(&json("[]")));
    assert_eq!(defaults.get("routing_strategy_args"), Some(&json("{}")));
    assert_eq!(defaults.get("num_retries"), Some(&json("0")));
    assert_eq!(defaults.get("max_fallbacks"), Some(&json("5")));
    assert_eq!(defaults.get("retry_after"), Some(&json("0")));
    assert_eq!(defaults.get("enable_pre_call_checks"), Some(&json("false")));
    assert_eq!(defaults.get("tag_filtering_match_any"), Some(&json("true")));
    assert_eq!(defaults.get("timeout"), Some(&json("null")));
    assert_eq!(field_default("num_retries"), json("0"));
    assert_eq!(field_default("not_a_router_setting"), json("null"));
}

#[test]
fn settings_endpoint_reports_the_router_defaults() {
    let answer = response("router_settings: {}");
    assert_eq!(answer.current_values.len(), 22);
    assert_eq!(
        answer.current_values.get("default_litellm_params"),
        Some(&json(
            r#"{"max_retries": 0, "metadata": {"caching_groups": null}, "timeout": null}"#
        ))
    );
    assert_eq!(
        answer.current_values.get("enable_tag_filtering"),
        Some(&json("false"))
    );
    assert_eq!(
        answer.current_values.get("routing_strategy_args"),
        Some(&json("{}"))
    );
}

#[test]
fn litellm_settings_feed_the_router_defaults() {
    let document = "litellm_settings: {num_retries: 6, fallbacks: [{gpt-4: [gpt-3.5-turbo]}]}";
    let answer = response(document);
    assert_eq!(answer.current_values.get("num_retries"), Some(&json("6")));
    assert_eq!(
        answer.current_values.get("fallbacks"),
        Some(&json(r#"[{"gpt-4": ["gpt-3.5-turbo"]}]"#))
    );
    assert_eq!(
        answer.source.get("num_retries"),
        Some(&FieldSource::Default)
    );
}

#[test]
fn default_fallbacks_append_to_the_configured_chains() {
    let document = "router_settings: {default_fallbacks: [gpt-3.5-turbo]}";
    let answer = response(document);
    assert_eq!(
        answer.current_values.get("fallbacks"),
        Some(&json(r#"[{"*": ["gpt-3.5-turbo"]}]"#))
    );
    assert_eq!(answer.current_values.get("default_fallbacks"), None);
}

#[test]
fn routing_groups_keep_their_configured_shape() {
    let document = "router_settings:\n  routing_groups:\n    - group_name: fast\n      models: [gpt-4]\n      routing_strategy: priority\n      model_priorities: {gpt-4: 1}\n    - group_name: slow\n      models: [gpt-3.5-turbo]\n      routing_strategy: simple-shuffle\n      routing_strategy_args: {ttl: 60}\n";
    let answer = response(document);
    assert_eq!(
        answer.current_values.get("routing_groups"),
        Some(&json(
            r#"[{"group_name": "fast", "models": ["gpt-4"], "routing_strategy": "priority", "model_priorities": {"gpt-4": 1}}, {"group_name": "slow", "models": ["gpt-3.5-turbo"], "routing_strategy": "simple-shuffle", "routing_strategy_args": {"ttl": 60}}]"#
        ))
    );
    assert_eq!(
        answer.source.get("routing_groups"),
        Some(&FieldSource::Config)
    );
}

#[test]
fn latency_arguments_are_reported_as_configured() {
    let document = "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args: {ttl: 60, lowest_latency_buffer: 0.1, max_latency_list_size: 2048, ttft_percentile: 0.95}\n";
    let answer = response(document);
    assert_eq!(
        answer.current_values.get("routing_strategy_args"),
        Some(&json(
            r#"{"ttl": 60, "lowest_latency_buffer": 0.1, "max_latency_list_size": 2048, "ttft_percentile": 0.95}"#
        ))
    );
    assert_eq!(
        answer.source.get("routing_strategy_args"),
        Some(&FieldSource::Config)
    );
}

#[test]
fn usage_arguments_are_reported_as_configured() {
    let document = "router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args: {ttl: 120}\n";
    let answer = response(document);
    assert_eq!(
        answer.current_values.get("routing_strategy_args"),
        Some(&json(r#"{"ttl": 120}"#))
    );
    assert_eq!(
        answer.source.get("routing_strategy_args"),
        Some(&FieldSource::Config)
    );
}

#[test]
fn lar1_thresholds_are_reported_as_configured() {
    let document = "router_settings:\n  routing_strategy: lar1\n  routing_strategy_args: {confidence_threshold_low: 0.4, confidence_threshold_medium: 0.6, confidence_threshold_high: 0.8}\n";
    let answer = response(document);
    assert_eq!(
        answer.current_values.get("routing_strategy_args"),
        Some(&json(
            r#"{"confidence_threshold_low": 0.4, "confidence_threshold_medium": 0.6, "confidence_threshold_high": 0.8}"#
        ))
    );
    assert_eq!(
        answer.source.get("routing_strategy_args"),
        Some(&FieldSource::Config)
    );
}

#[test]
fn configured_default_params_replace_the_computed_object() {
    let document =
        "router_settings:\n  timeout: 12.5\n  default_litellm_params: {stream_timeout: 1.5}\n";
    let answer = response(document);
    assert_eq!(
        answer.current_values.get("default_litellm_params"),
        Some(&json(r#"{"stream_timeout": 1.5}"#))
    );
    assert_eq!(
        answer.source.get("default_litellm_params"),
        Some(&FieldSource::Config)
    );
}

#[test]
fn an_unset_router_timeout_leaves_the_computed_default_params_timed_by_the_global() {
    let document = "litellm_settings: {request_timeout: 45.5}";
    let answer = response(document);
    assert_eq!(answer.current_values.get("timeout"), Some(&json("45.5")));
    assert_eq!(
        answer.current_values.get("default_litellm_params"),
        Some(&json(
            r#"{"max_retries": 0, "metadata": {"caching_groups": null}, "timeout": null}"#
        ))
    );
}

#[test]
fn a_written_whole_float_is_reported_as_an_integer() {
    let answer = response("router_settings: {timeout: 5.0}");
    assert_eq!(answer.current_values.get("timeout"), Some(&json("5")));
    assert_eq!(answer.source.get("timeout"), Some(&FieldSource::Config));
}

#[test]
fn a_configured_disable_cooldowns_false_reads_back_as_null() {
    let answer = response("router_settings: {disable_cooldowns: false}");
    assert_eq!(
        answer.current_values.get("disable_cooldowns"),
        Some(&json("null"))
    );
    assert_eq!(
        answer.source.get("disable_cooldowns"),
        Some(&FieldSource::Unset)
    );
}

#[test]
fn an_explicitly_empty_fallback_list_reads_back_as_null() {
    let answer = response("router_settings: {fallbacks: []}");
    assert_eq!(answer.current_values.get("fallbacks"), Some(&json("null")));
    assert_eq!(answer.source.get("fallbacks"), Some(&FieldSource::Default));
}

#[test]
fn caching_groups_is_unsupported_configuration() {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    let error = loader
        .load("router_settings: {caching_groups: [[gpt-4, gpt-3.5-turbo]]}")
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        r#"unsupported config keys: ["router_settings.caching_groups"]"#
    );
}

fn application(config: &Config) -> Router {
    router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new(),
        Router::new(),
    )
}

#[rstest]
#[case::settings("/router/settings")]
#[case::fields("/router/fields")]
#[tokio::test]
async fn router_settings_routes_reject_an_invalid_key(#[case] path: &str) {
    let config = load("general_settings:\n  master_key: secret\n  public_routes: []\n");
    let response = application(&config)
        .oneshot(
            Request::get(path)
                .header("authorization", "Bearer rejected")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
    let body: ErrorResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body.error.error_type, "no_db_connection");
    assert_eq!(body.error.param, None);
    assert_eq!(body.error.code, "400");
}

#[rstest]
#[case::settings("/router/settings", "current_values")]
#[case::fields("/router/fields", "routing_group_strategies")]
#[tokio::test]
async fn router_settings_routes_answer_the_master_key(#[case] path: &str, #[case] marker: &str) {
    let config = load("general_settings:\n  master_key: secret\n  public_routes: []\n");
    let response = application(&config)
        .oneshot(
            Request::get(path)
                .header("authorization", "Bearer secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains(marker), "{body}");
}

#[rstest]
#[case::settings("/router/settings")]
#[case::fields("/router/fields")]
#[tokio::test]
async fn router_settings_routes_inherit_the_shared_missing_key_failure(#[case] path: &str) {
    let config = load("general_settings:\n  master_key: secret\n  public_routes: []\n");
    let response = application(&config)
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
    assert_eq!(&*bytes, b"Internal Server Error");
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: ErrorFields,
}

#[derive(Deserialize)]
struct ErrorFields {
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: String,
}
