#![cfg(test)]

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::super::SettingsMap;
use super::super::body::decode_config_update;
use super::super::router_settings;
use super::support::{object, parsed, settings};

const FULL_BODY: &str = r#"{
    "model_list": [],
    "router_settings": {
        "routing_strategy": "priority",
        "routing_strategy_args": {"a": 1},
        "routing_groups": [{"group_name": "g", "models": ["m"], "routing_strategy": "simple-shuffle"}],
        "retry_policy": {"RateLimitErrorRetries": 2, "Unknown": 3},
        "model_group_retry_policy": {"g": {"DefaultRetries": 1}},
        "model_group_affinity_config": {"g": ["m"]},
        "allowed_fails": 3,
        "cooldown_time": 5,
        "num_retries": 2,
        "timeout": 30,
        "max_retries": 1,
        "retry_after": 0.5,
        "fallbacks": [{"g": ["m"]}],
        "context_window_fallbacks": [{"g": ["m"]}],
        "model_group_alias": {"alias": "g", "other": {"m": "g"}},
        "enable_tag_filtering": true,
        "weights": {"g": {"m": 1.5}},
        "tag_routing_prefix": "tag-",
        "optional_pre_call_checks": ["prompt_caching", "session_affinity"]
    }
}"#;

#[test]
fn decodes_every_declared_router_setting() {
    let decoded = decode_config_update(FULL_BODY.as_bytes()).unwrap();
    let typed = router_settings::typed_settings(decoded.router_settings.as_ref().unwrap()).unwrap();
    let expected = object(
        r#"{
            "routing_strategy": "priority",
            "routing_strategy_args": {"a": 1},
            "routing_groups": [{"group_name": "g", "models": ["m"], "routing_strategy": "simple-shuffle"}],
            "retry_policy": {"RateLimitErrorRetries": 2},
            "model_group_retry_policy": {"g": {"DefaultRetries": 1}},
            "model_group_affinity_config": {"g": ["m"]},
            "allowed_fails": 3,
            "cooldown_time": 5.0,
            "num_retries": 2,
            "timeout": 30.0,
            "max_retries": 1,
            "retry_after": 0.5,
            "fallbacks": [{"g": ["m"]}],
            "context_window_fallbacks": [{"g": ["m"]}],
            "model_group_alias": {"alias": "g", "other": {"m": "g"}},
            "enable_tag_filtering": true,
            "weights": {"g": {"m": 1.5}},
            "tag_routing_prefix": "tag-",
            "optional_pre_call_checks": ["prompt_caching", "session_affinity"]
        }"#,
    );
    assert_eq!(typed, expected);
}

#[test]
fn keeps_the_raw_router_settings_separate_from_the_typed_model() {
    let decoded = decode_config_update(
        br#"{"router_settings": {"weights": null, "enable_weighted_failover": true, "custom": 1}}"#,
    )
    .unwrap();
    let raw = decoded.raw_router_settings.as_ref().unwrap();
    assert_eq!(raw.get("weights"), Some(&JsonValue::Null));
    assert_eq!(raw.get("custom"), Some(&parsed("1")));
    let typed = router_settings::typed_settings(decoded.router_settings.as_ref().unwrap()).unwrap();
    assert!(!typed.contains_key("enable_weighted_failover"));
    assert!(typed.is_empty());
}

#[rstest]
#[case::allowed_fails("allowed_fails", r#""three""#)]
#[case::timeout("timeout", r#""soon""#)]
#[case::routing_strategy("routing_strategy", "1")]
#[case::strategy_args("routing_strategy_args", "[]")]
#[case::routing_groups("routing_groups", r#""g""#)]
#[case::retry_policy("retry_policy", r#""two""#)]
#[case::group_retry_policy("model_group_retry_policy", r#"{"g": 2}"#)]
#[case::affinity("model_group_affinity_config", r#"{"g": "m"}"#)]
#[case::fallbacks("fallbacks", r#"["g"]"#)]
#[case::alias("model_group_alias", r#"{"a": 1}"#)]
#[case::weights_negative("weights", r#"{"g": {"m": -1.0}}"#)]
#[case::weights_negative_with_a_positive("weights", r#"{"g": {"m": -1.0, "n": 1.0}}"#)]
#[case::weights_blank_group("weights", r#"{" ": {"m": 1.0}}"#)]
#[case::weights_no_positive("weights", r#"{"g": {"m": 0.0}}"#)]
#[case::pre_call("optional_pre_call_checks", r#"["nope"]"#)]
#[case::tag_prefix("tag_routing_prefix", "1")]
fn rejects_a_wrongly_typed_router_setting(#[case] field: &str, #[case] value: &str) {
    let body = format!(r#"{{"router_settings": {{"{field}": {value}}}}}"#);
    drop(decode_config_update(body.as_bytes()).unwrap_err());
}

#[test]
fn accepts_weights_with_a_zero_weight_and_an_empty_group() {
    drop(
        decode_config_update(
            br#"{"router_settings": {"weights": {"empty": {}, "g": {"m": 0.0, "n": 1.0}}}}"#,
        )
        .unwrap(),
    );
}

#[test]
fn reads_a_null_section_as_absent() {
    let decoded = decode_config_update(
        br#"{"general_settings": null, "router_settings": null, "litellm_settings": {}, "environment_variables": {"A": "b"}}"#,
    )
    .unwrap();
    assert!(decoded.general_settings.is_none());
    assert!(decoded.router_settings.is_none());
    assert!(decoded.raw_router_settings.is_none());
    assert_eq!(decoded.litellm_settings, Some(SettingsMap::new()));
    assert_eq!(
        decoded.environment_variables,
        Some(settings(&[("A", JsonValue::String("b".to_owned()))]))
    );
}

#[test]
fn rejects_a_missing_or_non_object_body() {
    drop(decode_config_update(b"").unwrap_err());
    drop(decode_config_update(b"null").unwrap_err());
    drop(decode_config_update(b"[]").unwrap_err());
    drop(decode_config_update(br#"{"router_settings": "x"}"#).unwrap_err());
    drop(decode_config_update(br#"{"general_settings": []}"#).unwrap_err());
}
