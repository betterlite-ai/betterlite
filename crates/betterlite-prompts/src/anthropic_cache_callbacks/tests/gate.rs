use rstest::rstest;

use crate::anthropic_cache_callbacks::gate::{
    cache_control_custom_logger, should_use_cache_control,
};
use crate::arguments::json::Object;

use super::support::decode;

const ONE_POINT: &str =
    r#"{"cache_control_injection_points": [{"location": "tool_config", "control": null}]}"#;

#[rstest]
#[case::no_key("{}")]
#[case::a_null_value(r#"{"cache_control_injection_points": null}"#)]
#[case::an_empty_list(r#"{"cache_control_injection_points": []}"#)]
#[case::an_empty_map(r#"{"cache_control_injection_points": {}}"#)]
#[case::an_empty_string(r#"{"cache_control_injection_points": ""}"#)]
#[case::a_zero(r#"{"cache_control_injection_points": 0}"#)]
#[case::a_false(r#"{"cache_control_injection_points": false}"#)]
fn cache_control_is_not_requested(#[case] params: &str) {
    assert!(!should_use_cache_control(&decode::<Object>(params)));
}

#[rstest]
#[case::a_point_list(ONE_POINT)]
#[case::a_non_empty_map(r#"{"cache_control_injection_points": {"location": "tool_config"}}"#)]
#[case::a_non_empty_string(r#"{"cache_control_injection_points": "default"}"#)]
#[case::a_one(r#"{"cache_control_injection_points": 1}"#)]
fn cache_control_is_requested(#[case] params: &str) {
    assert!(should_use_cache_control(&decode::<Object>(params)));
}

#[test]
fn the_logger_factory_returns_the_logger_when_cache_control_is_requested() {
    let params = decode::<Object>(ONE_POINT);
    let mut names = Vec::new();
    let logger = cache_control_custom_logger(&params, |name| {
        names.push(name);
        Some("custom-logger")
    });
    assert_eq!(names, vec!["anthropic_cache_control_hook"]);
    assert_eq!(logger, Some("custom-logger"));
}

#[test]
fn the_logger_factory_skips_the_init_function_without_cache_control() {
    let params = decode::<Object>("{}");
    let mut called = false;
    let logger = cache_control_custom_logger(&params, |_| {
        called = true;
        Some("custom-logger")
    });
    assert!(!called);
    assert_eq!(logger, None);
}

#[test]
fn the_logger_factory_returns_what_the_init_function_gives() {
    let params = decode::<Object>(ONE_POINT);
    let logger: Option<&str> = cache_control_custom_logger(&params, |_| None);
    assert_eq!(logger, None);
}
