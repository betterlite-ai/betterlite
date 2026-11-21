#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::settings::{FieldSource, JsonValue};
use betterlite_config::types::Config;
use rstest::rstest;
use serde_json::json;

use super::response::{RouterSettingsResponse, fields_response, settings_response};
use super::view::{current_values, settings_store};

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

fn value(document: &str, key: &str) -> JsonValue {
    response(document).current_values.get(key).cloned().unwrap()
}

#[rstest]
#[case::strategy_default("router_settings: {}", "routing_strategy", json!("simple-shuffle"), FieldSource::Default)]
#[case::strategy_configured(
    "router_settings: {routing_strategy: least-busy}",
    "routing_strategy",
    json!("least-busy"),
    FieldSource::Config
)]
#[case::retries_default("router_settings: {}", "num_retries", json!(2), FieldSource::Default)]
#[case::retries_configured(
    "router_settings: {num_retries: 4}",
    "num_retries",
    json!(4),
    FieldSource::Config
)]
#[case::timeout_default("router_settings: {}", "timeout", json!(6000.0), FieldSource::Default)]
#[case::timeout_configured(
    "router_settings: {timeout: 30.5}",
    "timeout",
    json!(30.5),
    FieldSource::Config
)]
#[case::stream_timeout_unset("router_settings: {}", "stream_timeout", json!(null), FieldSource::Unset)]
#[case::stream_timeout_configured(
    "router_settings: {stream_timeout: 7.5}",
    "stream_timeout",
    json!(7.5),
    FieldSource::Config
)]
#[case::max_fallbacks_default("router_settings: {}", "max_fallbacks", json!(5), FieldSource::Default)]
#[case::fallbacks_unset("router_settings: {}", "fallbacks", json!(null), FieldSource::Default)]
#[case::fallbacks_configured(
    "router_settings: {fallbacks: [{gpt-4: [gpt-3.5-turbo]}]}",
    "fallbacks",
    json!([{"gpt-4": ["gpt-3.5-turbo"]}]),
    FieldSource::Config
)]
#[case::context_window_default(
    "router_settings: {}",
    "context_window_fallbacks",
    json!(null),
    FieldSource::Default
)]
#[case::context_window_configured(
    "router_settings: {context_window_fallbacks: [{gpt-4: [gpt-3.5-turbo]}]}",
    "context_window_fallbacks",
    json!([{"gpt-4": ["gpt-3.5-turbo"]}]),
    FieldSource::Config
)]
#[case::content_policy_configured(
    "router_settings: {content_policy_fallbacks: [{gpt-4: [gpt-3.5-turbo]}]}",
    "content_policy_fallbacks",
    json!([{"gpt-4": ["gpt-3.5-turbo"]}]),
    FieldSource::Config
)]
#[case::allowed_fails_default("router_settings: {}", "allowed_fails", json!(3), FieldSource::Default)]
#[case::allowed_fails_zero(
    "router_settings: {allowed_fails: 0}",
    "allowed_fails",
    json!(0),
    FieldSource::Config
)]
#[case::cooldown_default("router_settings: {}", "cooldown_time", json!(5), FieldSource::Default)]
#[case::cooldown_configured(
    "router_settings: {cooldown_time: 2.5}",
    "cooldown_time",
    json!(2.5),
    FieldSource::Config
)]
#[case::retry_after_default("router_settings: {}", "retry_after", json!(0), FieldSource::Default)]
#[case::retry_policy_unset("router_settings: {}", "retry_policy", json!(null), FieldSource::Unset)]
#[case::retry_policy_configured(
    "router_settings: {retry_policy: {BadRequestErrorRetries: 2}}",
    "retry_policy",
    json!({"BadRequestErrorRetries": 2}),
    FieldSource::Config
)]
#[case::alias_unset("router_settings: {}", "model_group_alias", json!({}), FieldSource::Default)]
#[case::alias_configured(
    "router_settings: {model_group_alias: {gpt-4: gpt-3.5-turbo}}",
    "model_group_alias",
    json!({"gpt-4": "gpt-3.5-turbo"}),
    FieldSource::Config
)]
#[case::pre_call_checks_default("router_settings: {}", "enable_pre_call_checks", json!(false), FieldSource::Default)]
#[case::pre_call_checks_configured(
    "router_settings: {enable_pre_call_checks: true}",
    "enable_pre_call_checks",
    json!(true),
    FieldSource::Config
)]
#[case::max_parallel_unset("router_settings: {}", "default_max_parallel_requests", json!(null), FieldSource::Unset)]
#[case::max_parallel_configured(
    "router_settings: {default_max_parallel_requests: 8}",
    "default_max_parallel_requests",
    json!(8),
    FieldSource::Config
)]
#[case::verbose_default("router_settings: {}", "set_verbose", json!(false), FieldSource::Default)]
#[case::tag_filtering_default("router_settings: {}", "tag_filtering_match_any", json!(true), FieldSource::Default)]
#[case::cooldowns_unset("router_settings: {}", "disable_cooldowns", json!(null), FieldSource::Unset)]
#[case::groups_unset("router_settings: {}", "routing_groups", json!([]), FieldSource::Default)]
#[case::strategy_args_unset(
    "router_settings: {}",
    "routing_strategy_args",
    json!({}),
    FieldSource::Default
)]
fn router_settings_behaviors(
    #[case] document: &str,
    #[case] key: &str,
    #[case] expected: JsonValue,
    #[case] expected_source: FieldSource,
) {
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
#[case::zero_retries("router_settings: {num_retries: 0}", "num_retries", json!(0))]
#[case::zero_fails("router_settings: {allowed_fails: 0}", "allowed_fails", json!(0))]
#[case::zero_max_fallbacks("router_settings: {max_fallbacks: 0}", "max_fallbacks", json!(0))]
#[case::zero_cooldown("router_settings: {cooldown_time: 0}", "cooldown_time", json!(0))]
#[case::zero_timeout("router_settings: {timeout: 0}", "timeout", json!(0))]
#[case::largest_allowed_fails(
    "router_settings: {allowed_fails: 4294967295}",
    "allowed_fails",
    json!(4294967295_u32)
)]
#[case::largest_max_fallbacks(
    "router_settings: {max_fallbacks: 9223372036854775807}",
    "max_fallbacks",
    json!(9223372036854775807_i64)
)]
#[case::empty_alias("router_settings: {model_group_alias: {gpt-4: ''}}", "model_group_alias", json!({"gpt-4": ""}))]
#[case::empty_groups("router_settings: {routing_groups: []}", "routing_groups", json!([]))]
#[case::fractional_retry_after(
    "router_settings: {retry_after: 0.5}",
    "retry_after",
    json!(0.5)
)]
fn router_settings_boundaries(
    #[case] document: &str,
    #[case] key: &str,
    #[case] expected: JsonValue,
) {
    assert_eq!(value(document, key), expected);
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
    let tag = answer
        .fields
        .iter()
        .find(|field| field.field_name == "enable_tag_filtering")
        .unwrap();
    assert_eq!(tag.link, Some("https://docs.litellm.ai/docs/tag_routing"));
    assert!(
        answer
            .fields
            .iter()
            .filter(|field| field.field_name != "enable_tag_filtering")
            .all(|field| field.link.is_none())
    );
}

#[test]
fn settings_endpoint_reports_the_router_defaults() {
    let answer = response("router_settings: {}");
    assert_eq!(answer.current_values.len(), 22);
    assert_eq!(
        answer.current_values.get("default_litellm_params"),
        Some(&json!({
            "max_retries": 0,
            "metadata": {"caching_groups": null},
            "timeout": null,
        }))
    );
    assert_eq!(
        answer.current_values.get("enable_tag_filtering"),
        Some(&json!(false))
    );
    assert_eq!(
        answer.current_values.get("routing_strategy_args"),
        Some(&json!({}))
    );
}

#[test]
fn litellm_settings_feed_the_router_defaults() {
    let document = "litellm_settings: {num_retries: 6, fallbacks: [{gpt-4: [gpt-3.5-turbo]}]}";
    let answer = response(document);
    assert_eq!(answer.current_values.get("num_retries"), Some(&json!(6)));
    assert_eq!(
        answer.current_values.get("fallbacks"),
        Some(&json!([{"gpt-4": ["gpt-3.5-turbo"]}]))
    );
    assert_eq!(
        answer.source.get("num_retries"),
        Some(&FieldSource::Default)
    );
}

#[test]
fn default_fallbacks_append_to_the_configured_chains() {
    let document = "router_settings: {default_fallbacks: [gpt-3.5-turbo]}";
    assert_eq!(
        value(document, "fallbacks"),
        json!([{"*": ["gpt-3.5-turbo"]}])
    );
}

#[test]
fn routing_groups_keep_their_configured_shape() {
    let document = "router_settings:\n  routing_groups:\n    - group_name: fast\n      models: [gpt-4]\n      routing_strategy: priority\n      model_priorities: {gpt-4: 1}\n    - group_name: slow\n      models: [gpt-3.5-turbo]\n      routing_strategy: simple-shuffle\n      routing_strategy_args: {ttl: 60}\n";
    let answer = response(document);
    assert_eq!(
        answer.current_values.get("routing_groups"),
        Some(&json!([
            {
                "group_name": "fast",
                "models": ["gpt-4"],
                "routing_strategy": "priority",
                "model_priorities": {"gpt-4": 1},
            },
            {
                "group_name": "slow",
                "models": ["gpt-3.5-turbo"],
                "routing_strategy": "simple-shuffle",
                "routing_strategy_args": {"ttl": 60},
            },
        ]))
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
        Some(&json!({
            "ttl": 60,
            "lowest_latency_buffer": 0.1,
            "max_latency_list_size": 2048,
            "ttft_percentile": 0.95,
        }))
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
        Some(&json!({"ttl": 120}))
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
        Some(&json!({
            "confidence_threshold_low": 0.4,
            "confidence_threshold_medium": 0.6,
            "confidence_threshold_high": 0.8,
        }))
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
        Some(&json!({"stream_timeout": 1.5}))
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
    assert_eq!(answer.current_values.get("timeout"), Some(&json!(45.5)));
    assert_eq!(
        answer.current_values.get("default_litellm_params"),
        Some(&json!({
            "max_retries": 0,
            "metadata": {"caching_groups": null},
            "timeout": null,
        }))
    );
}
