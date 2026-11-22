use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;

use super::*;
use crate::responses_affinity::config::AFFINITY_CHECK;

fn group_settings(group: &str) -> AffinitySettings {
    settings(false, &[(group, &[AFFINITY_CHECK])])
}

#[test]
fn a_disabled_check_returns_the_candidates_unchanged() {
    let deployments = vec![
        spec("A", params(Some("u-a"), Some("k-a"))),
        spec("B", params(Some("u-b"), Some("k-b"))),
    ];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(false, &[]), &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(ids(&returned), ids(&deployments));
    assert!(!request.pinned);
    assert_eq!(request.metadata, None);
    assert_eq!(request.input, Some(Json::List(vec![reasoning_item("A")])));
}

#[test]
fn a_disabled_check_does_not_touch_a_non_object_metadata() {
    let deployments = vec![spec("A", params(Some("u"), Some("k")))];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(false, &[]), &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    request.metadata = Some(Json::Null);
    let returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(ids(&returned), ids(&deployments));
    assert_eq!(request.metadata, Some(Json::Null));
}

#[test]
fn global_affinity_enables_every_group() {
    let deployments = vec![spec("A", params(Some("u"), Some("k")))];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let returned = request.filter(&check, "any-group", &deployments).unwrap();
    assert_eq!(ids(&returned), vec![Some("A".to_owned())]);
    assert!(request.pinned);
}

#[test]
fn a_group_listed_with_the_literal_check_activates_without_the_global_switch() {
    let deployments = vec![spec("A", params(Some("u"), Some("k")))];
    let credentials = CredentialSnapshot::default();
    let check = check(group_settings("gpt-group"), &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(ids(&returned), vec![Some("A".to_owned())]);
    assert!(request.pinned);
}

#[test]
fn another_groups_list_does_not_enable_this_group() {
    let deployments = vec![spec("A", params(Some("u"), Some("k")))];
    let credentials = CredentialSnapshot::default();
    let check = check(group_settings("other-group"), &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(ids(&returned), ids(&deployments));
    assert!(!request.pinned);
}

#[test]
fn a_group_listed_without_the_literal_check_stays_disabled() {
    let deployments = vec![spec("A", params(Some("u"), Some("k")))];
    let credentials = CredentialSnapshot::default();
    let listed = settings(false, &[("gpt-group", &["other_check"])]);
    let check = check(listed, &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(ids(&returned), ids(&deployments));
    assert!(!request.pinned);
}

#[test]
fn has_group_affinity_enabled_reads_every_group_list() {
    assert!(group_settings("gpt-group").has_group_affinity_enabled());
    assert!(!settings(false, &[]).has_group_affinity_enabled());
    assert!(!settings(true, &[]).has_group_affinity_enabled());
    assert!(!settings(false, &[("gpt-group", &["other_check"])]).has_group_affinity_enabled());
}

#[test]
fn enabled_for_group_follows_the_global_switch_and_the_group_list() {
    assert!(settings(true, &[]).enabled_for_group("anything"));
    assert!(group_settings("gpt-group").enabled_for_group("gpt-group"));
    assert!(!group_settings("other-group").enabled_for_group("gpt-group"));
}

#[test]
fn default_settings_follow_the_constructor() {
    let settings = AffinitySettings::default();
    assert!(settings.enable_global_affinity);
    assert!(settings.model_group_affinity_config.is_empty());
    assert!(!settings.has_group_affinity_enabled());
    assert!(settings.enabled_for_group("any-group"));
}

#[test]
fn every_json_shape_has_its_type_error_name() {
    assert_eq!(error_type_name(&Json::Null), "NoneType");
    assert_eq!(error_type_name(&Json::Bool(true)), "bool");
    assert_eq!(
        error_type_name(&Json::Number(serde_json::Number::from(1_i32))),
        "int"
    );
    let float = serde_json::Number::from_f64(0.5_f64).unwrap();
    assert_eq!(error_type_name(&Json::Number(float)), "float");
    assert_eq!(error_type_name(&text("value")), "str");
    assert_eq!(error_type_name(&Json::List(Vec::new())), "list");
    assert_eq!(error_type_name(&Json::Object(JsonObject::new())), "dict");
}

#[test]
fn the_flag_is_written_into_existing_metadata() {
    let deployments = vec![spec("A", params(Some("u"), Some("k")))];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::new();
    request.metadata = Some(Json::Object(object(&[(
        "tags",
        Json::List(vec![text("routing-tag")]),
    )])));
    let _returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    let expected = Some(Json::Object(object(&[
        ("tags", Json::List(vec![text("routing-tag")])),
        ("encrypted_content_affinity_enabled", Json::Bool(true)),
    ])));
    assert_eq!(request.metadata, expected);
}

#[test]
fn a_missing_metadata_is_never_created() {
    let deployments = vec![spec("A", params(Some("u"), Some("k")))];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::with_items(vec![reasoning_item("A")]);
    let returned = request.filter(&check, "gpt-group", &deployments).unwrap();
    assert_eq!(ids(&returned), vec![Some("A".to_owned())]);
    assert_eq!(request.metadata, None);
}

#[test]
fn a_null_metadata_raises_pythons_type_error() {
    let deployments = vec![spec("A", params(Some("u"), Some("k")))];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::new();
    request.metadata = Some(Json::Null);
    let error = request
        .filter(&check, "gpt-group", &deployments)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "'NoneType' object does not support item assignment"
    );
}

#[test]
fn a_negative_number_metadata_raises_pythons_type_error() {
    let deployments = vec![spec("A", params(Some("u"), Some("k")))];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::new();
    request.metadata = Some(Json::Number(serde_json::Number::from(-1_i32)));
    let error = request
        .filter(&check, "gpt-group", &deployments)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "'int' object does not support item assignment"
    );
}

#[test]
fn a_text_metadata_raises_pythons_type_error() {
    let deployments = vec![spec("A", params(Some("u"), Some("k")))];
    let credentials = CredentialSnapshot::default();
    let check = check(settings(true, &[]), &credentials);
    let mut request = Request::new();
    request.metadata = Some(text("metadata"));
    let error = request
        .filter(&check, "gpt-group", &deployments)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "'str' object does not support item assignment"
    );
}
