#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::TagError;
use super::TagSettings;
use super::get_extra_header_tags;
use super::get_request_tags;
use super::get_user_agent_tags;
use crate::logging_metadata::params::LitellmParams;

type Headers = IndexMap<String, String>;

fn params(encoded: &str) -> LitellmParams {
    serde_json::from_str(encoded).expect("params decode")
}

fn headers(encoded: &str) -> Headers {
    serde_json::from_str(encoded).expect("headers decode")
}

fn settings(extra: &[String], disabled: bool) -> TagSettings<'_> {
    TagSettings {
        disable_user_agent_tags: disabled,
        extra_spend_tag_headers: extra,
    }
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[rstest]
#[case::with_slash(r#"{"user-agent": "curl/8.0"}"#, vec![text("User-Agent: curl"), text("User-Agent: curl/8.0")])]
#[case::without_slash(r#"{"user-agent": "curl"}"#, vec![text("User-Agent: curl")])]
#[case::empty(r#"{"user-agent": ""}"#, vec![text("User-Agent: ")])]
fn logging_metadata_behavior_tests_behavior(#[case] encoded: &str, #[case] expected: Vec<Json>) {
    assert_eq!(
        get_user_agent_tags(Some(&headers(encoded)), false),
        Some(expected)
    );
}

#[test]
fn absent_or_disabled_user_agent_tags_stay_absent() {
    assert_eq!(get_user_agent_tags(None, false), None);
    assert_eq!(
        get_user_agent_tags(Some(&headers(r#"{"user-agent": "curl/8"}"#)), true),
        None
    );
    assert_eq!(get_user_agent_tags(Some(&headers("{}")), false), None);
}

#[test]
fn extra_header_tags_follow_the_configured_names() {
    let extra = vec!["x-user".to_owned(), "x-team".to_owned()];
    let tags = get_extra_header_tags(Some(&headers(r#"{"x-user": "ada", "x-team": ""}"#)), &extra);
    assert_eq!(tags, Some(vec![text("x-user: ada"), text("x-team: ")]));
}

#[test]
fn extra_header_tags_are_absent_without_configuration_or_headers() {
    assert_eq!(
        get_extra_header_tags(Some(&headers(r#"{"x-user": "ada"}"#)), &[]),
        None
    );
    assert_eq!(get_extra_header_tags(None, &["x-user".to_owned()]), None);
    assert_eq!(
        get_extra_header_tags(Some(&headers("{}")), &["x-user".to_owned()]),
        None
    );
}

#[test]
fn request_tags_come_from_metadata_first() {
    let params = params(r#"{"metadata": {"tags": ["a"]}, "litellm_metadata": {"tags": ["b"]}}"#);
    let tags = get_request_tags(&params, None, &settings(&[], false)).expect("tags");
    assert_eq!(tags, vec![text("a")]);
}

#[test]
fn request_tags_fall_back_to_litellm_metadata() {
    let params = params(r#"{"litellm_metadata": {"tags": ["b"]}}"#);
    let tags = get_request_tags(&params, None, &settings(&[], false)).expect("tags");
    assert_eq!(tags, vec![text("b")]);
}

#[test]
fn an_explicit_empty_tag_list_wins_over_the_other_source() {
    let params = params(r#"{"metadata": {"tags": []}, "litellm_metadata": {"tags": ["b"]}}"#);
    let tags = get_request_tags(&params, None, &settings(&[], false)).expect("tags");
    assert!(tags.is_empty());
}

#[test]
fn request_tags_append_the_user_agent_and_configured_headers() {
    let params = params(r#"{"metadata": {"tags": ["a", 7]}}"#);
    let extra = vec!["x-team".to_owned()];
    let tags = get_request_tags(
        &params,
        Some(&headers(r#"{"user-agent": "curl/8", "x-team": "core"}"#)),
        &settings(&extra, false),
    )
    .expect("tags");
    assert_eq!(
        tags,
        vec![
            text("a"),
            Json::Number(serde_json::Number::from(7_i32)),
            text("User-Agent: curl"),
            text("User-Agent: curl/8"),
            text("x-team: core"),
        ]
    );
}

#[test]
fn a_non_list_tag_value_is_refused_by_field_name() {
    let params = params(r#"{"litellm_metadata": {"tags": "a"}}"#);
    let error = get_request_tags(&params, None, &settings(&[], false)).expect_err("wrong type");
    assert!(matches!(
        error,
        TagError::TagsType {
            field: "litellm_metadata"
        }
    ));
}

#[test]
fn request_tags_are_empty_without_any_source() {
    let params = params("{}");
    assert!(
        get_request_tags(&params, None, &settings(&[], false))
            .expect("tags")
            .is_empty()
    );
}
