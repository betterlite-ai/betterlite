#![cfg(test)]

pub(crate) mod support;

use betterlite_auth::auth_identity::roles::Role;
use betterlite_core::agents::json::AgentJson as JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use rstest::rstest;

use crate::auth::auth_key_lookup::tests::master_caller;

use super::guards::{CallerKeyMissing, MISSING_KEY_DETAIL, enforce_provider_key};
use super::merge::{
    TEMPLATE_FIELD, default_provider, merge_body_template, merge_query_params_into_data,
    query_values,
};

use support::{json_map, principal, query};

#[rstest]
#[case::proxy_admin(&[Role::PlatformAdmin], true)]
#[case::platform_viewer(&[Role::PlatformViewer], false)]
#[case::org_admin(&[Role::OrgAdmin], false)]
#[case::org_viewer(&[Role::OrgViewer], false)]
#[case::team_admin(&[Role::TeamAdmin], false)]
#[case::team_member(&[Role::TeamMember], false)]
#[case::no_roles(&[], false)]
fn only_the_proxy_admin_role_passes_the_key_gate(#[case] roles: &[Role], #[case] expected: bool) {
    let gate = enforce_provider_key(&JsonMap::new(), &principal("user-1", roles));
    assert_eq!(gate.is_ok(), expected);
}

#[test]
fn the_master_key_record_passes_the_key_gate_without_a_role() {
    enforce_provider_key(&JsonMap::new(), &master_caller().principal).unwrap();
}

#[test]
fn a_key_whose_subject_is_master_key_does_not_pass_the_key_gate() {
    let rejected = enforce_provider_key(&JsonMap::new(), &principal("master_key", &[]));
    assert_eq!(rejected, Err(CallerKeyMissing));
}

#[test]
fn another_subject_without_the_role_does_not_pass_the_key_gate() {
    let rejected = enforce_provider_key(&JsonMap::new(), &principal("admin-1", &[]));
    assert_eq!(rejected, Err(CallerKeyMissing));
}

#[test]
fn a_rejected_caller_gets_the_credential_detail_and_a_401() {
    let rejected = enforce_provider_key(&JsonMap::new(), &principal("user-1", &[]));
    assert_eq!(rejected, Err(CallerKeyMissing));
    assert_eq!(CallerKeyMissing::STATUS.as_u16(), 401);
    assert_eq!(CallerKeyMissing::DETAIL, MISSING_KEY_DETAIL);
}

#[rstest]
#[case::absent("{}", false)]
#[case::null(r#"{"api_key": null}"#, false)]
#[case::empty_string(r#"{"api_key": ""}"#, false)]
#[case::false_value(r#"{"api_key": false}"#, false)]
#[case::zero(r#"{"api_key": 0}"#, false)]
#[case::zero_float(r#"{"api_key": 0.0}"#, false)]
#[case::empty_array(r#"{"api_key": []}"#, false)]
#[case::empty_object(r#"{"api_key": {}}"#, false)]
#[case::text(r#"{"api_key": "AIza"}"#, true)]
#[case::true_value(r#"{"api_key": true}"#, true)]
#[case::nonzero(r#"{"api_key": 1}"#, true)]
#[case::array(r#"{"api_key": ["AIza"]}"#, true)]
#[case::object(r#"{"api_key": {"token": "AIza"}}"#, true)]
fn the_api_key_check_follows_python_truthiness(#[case] data: &str, #[case] expected: bool) {
    let checked = enforce_provider_key(&json_map(data), &principal("user-1", &[]));
    assert_eq!(checked.is_ok(), expected);
}

#[test]
fn an_admin_passes_without_a_key() {
    let checked = enforce_provider_key(
        &JsonMap::new(),
        &principal("admin-1", &[Role::PlatformAdmin]),
    );
    assert_eq!(checked, Ok(()));
}

#[test]
fn a_query_template_merges_its_entries() {
    let raw = query(&[(TEMPLATE_FIELD, r#"{"api_key":"AIza","project":"p1"}"#)]);
    let mut data = JsonMap::new();
    merge_query_params_into_data(&mut data, &query_values(Some(raw.as_str())));
    assert_eq!(data, json_map(r#"{"api_key": "AIza", "project": "p1"}"#));
}

#[test]
fn a_query_template_never_replaces_a_key_already_in_data() {
    let raw = query(&[(
        TEMPLATE_FIELD,
        r#"{"custom_llm_provider":"vertex","api_key":"AIza"}"#,
    )]);
    let mut data = json_map(r#"{"custom_llm_provider": "gemini"}"#);
    merge_query_params_into_data(&mut data, &query_values(Some(raw.as_str())));
    assert_eq!(
        data,
        json_map(r#"{"custom_llm_provider": "gemini", "api_key": "AIza"}"#)
    );
}

#[test]
fn a_query_template_does_not_replace_a_null_value() {
    let raw = query(&[(TEMPLATE_FIELD, r#"{"api_key":"AIza"}"#)]);
    let mut data = json_map(r#"{"api_key": null}"#);
    merge_query_params_into_data(&mut data, &query_values(Some(raw.as_str())));
    assert_eq!(data, json_map(r#"{"api_key": null}"#));
}

#[test]
fn a_malformed_query_template_merges_nothing() {
    let raw = query(&[(TEMPLATE_FIELD, "{not json")]);
    let mut data = json_map(r#"{"custom_llm_provider": "gemini"}"#);
    merge_query_params_into_data(&mut data, &query_values(Some(raw.as_str())));
    assert_eq!(data, json_map(r#"{"custom_llm_provider": "gemini"}"#));
}

#[rstest]
#[case::list("[1,2]")]
#[case::text("\"AIza\"")]
#[case::number("7")]
#[case::null("null")]
#[case::bare("true")]
fn a_decoded_query_template_that_is_not_an_object_merges_nothing(#[case] template: &str) {
    let raw = query(&[(TEMPLATE_FIELD, template)]);
    let mut data = JsonMap::new();
    merge_query_params_into_data(&mut data, &query_values(Some(raw.as_str())));
    assert!(data.is_empty());
}

#[test]
fn an_empty_query_template_merges_nothing() {
    let raw = query(&[(TEMPLATE_FIELD, "")]);
    let mut data = json_map(r#"{"custom_llm_provider": "gemini"}"#);
    merge_query_params_into_data(&mut data, &query_values(Some(raw.as_str())));
    assert_eq!(data, json_map(r#"{"custom_llm_provider": "gemini"}"#));
}

#[test]
fn a_flat_api_key_query_parameter_is_ignored() {
    let raw = query(&[("api_key", "AIza"), ("page", "2")]);
    let mut data = JsonMap::new();
    merge_query_params_into_data(&mut data, &query_values(Some(raw.as_str())));
    assert!(data.is_empty());
}

#[test]
fn a_repeated_query_template_keeps_its_last_value() {
    let raw = query(&[
        (TEMPLATE_FIELD, r#"{"api_key":"first"}"#),
        (TEMPLATE_FIELD, r#"{"api_key":"last"}"#),
    ]);
    let mut data = JsonMap::new();
    merge_query_params_into_data(&mut data, &query_values(Some(raw.as_str())));
    assert_eq!(
        data.get("api_key"),
        Some(&JsonValue::String("last".to_owned()))
    );
}

#[test]
fn a_query_template_value_is_percent_decoded() {
    let raw = query(&[(TEMPLATE_FIELD, r#"{"api_key":"AIza /+=&"}"#)]);
    let mut data = JsonMap::new();
    merge_query_params_into_data(&mut data, &query_values(Some(raw.as_str())));
    assert_eq!(
        data.get("api_key"),
        Some(&JsonValue::String("AIza /+=&".to_owned()))
    );
}

#[test]
fn a_query_string_without_the_template_leaves_data_alone() {
    let mut data = json_map(r#"{"custom_llm_provider": "gemini"}"#);
    merge_query_params_into_data(&mut data, &query_values(Some("page=2&limit=10")));
    assert_eq!(data, json_map(r#"{"custom_llm_provider": "gemini"}"#));
}

#[test]
fn a_body_template_fills_missing_keys_and_the_body_keeps_precedence() {
    let mut data =
        json_map(r#"{"name":"agent","litellm_params_template":{"name":"other","api_key":"AIza"}}"#);
    merge_body_template(&mut data);
    assert_eq!(
        data.get("name"),
        Some(&JsonValue::String("agent".to_owned()))
    );
    assert_eq!(
        data.get("api_key"),
        Some(&JsonValue::String("AIza".to_owned()))
    );
    assert!(!data.contains_key(TEMPLATE_FIELD));
}

#[rstest]
#[case::list("[1,2]")]
#[case::text("\"AIza\"")]
#[case::number("7")]
#[case::null("null")]
#[case::false_value("false")]
fn a_body_template_that_is_not_an_object_is_dropped_without_merging(#[case] template: &str) {
    let body = format!(r#"{{"litellm_params_template":{template}}}"#);
    let mut data = json_map(&body);
    merge_body_template(&mut data);
    assert!(data.is_empty());
}

#[test]
fn the_provider_defaults_to_gemini_after_the_merge() {
    let mut data = json_map(r#"{"litellm_params_template":{"api_key":"AIza"}}"#);
    merge_body_template(&mut data);
    default_provider(&mut data);
    assert_eq!(
        data.get("custom_llm_provider"),
        Some(&JsonValue::String("gemini".to_owned()))
    );
}

#[test]
fn a_template_provider_wins_over_the_gemini_default() {
    let mut data = json_map(r#"{"litellm_params_template":{"custom_llm_provider":"vertex"}}"#);
    merge_body_template(&mut data);
    default_provider(&mut data);
    assert_eq!(
        data.get("custom_llm_provider"),
        Some(&JsonValue::String("vertex".to_owned()))
    );
}

#[test]
fn a_body_provider_wins_over_the_template_and_the_default() {
    let mut data = json_map(
        r#"{"custom_llm_provider":"anthropic","litellm_params_template":{"custom_llm_provider":"vertex"}}"#,
    );
    merge_body_template(&mut data);
    default_provider(&mut data);
    assert_eq!(
        data.get("custom_llm_provider"),
        Some(&JsonValue::String("anthropic".to_owned()))
    );
}

#[test]
fn an_explicit_null_provider_is_kept_as_python_setdefault_keeps_it() {
    let mut data = json_map(r#"{"custom_llm_provider": null}"#);
    default_provider(&mut data);
    assert_eq!(data.get("custom_llm_provider"), Some(&JsonValue::Null));
}
