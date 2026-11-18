#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::{AccessGroupSources, coerce_model_access_groups, request_model_access_groups};

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("the fixture is a JSON object")
}

fn groups(text: &str) -> Vec<String> {
    let fixture = object(text);
    let sources = AccessGroupSources {
        standard_logging_object: fixture
            .get("standard_logging_object")
            .and_then(as_object_ref),
        litellm_params: fixture.get("litellm_params").and_then(as_object_ref),
        auth_groups: fixture.get("auth_matched_model_access_groups"),
    };
    request_model_access_groups(&sources)
}

fn as_object_ref(value: &Json) -> Option<&JsonObject> {
    match value {
        Json::Object(entries) => Some(entries),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

#[test]
fn the_standard_payload_wins_over_request_metadata() {
    assert_eq!(
        groups(
            r#"{
                "standard_logging_object": {"request_model_access_groups": ["payload", "payload"]},
                "litellm_params": {"metadata": {"user_api_key_matched_model_access_groups": ["stamped"]}}
            }"#
        ),
        vec!["payload".to_owned()]
    );
}

#[test]
fn an_empty_standard_payload_falls_through_to_the_metadata() {
    assert_eq!(
        groups(
            r#"{
                "standard_logging_object": {"request_model_access_groups": []},
                "litellm_params": {"metadata": {"user_api_key_matched_model_access_groups": ["stamped"]}}
            }"#
        ),
        vec!["stamped".to_owned()]
    );
}

#[test]
fn the_stamped_key_wins_over_the_auth_object() {
    assert_eq!(
        groups(
            r#"{
                "litellm_params": {"metadata": {
                    "user_api_key_matched_model_access_groups": ["stamped"],
                    "user_api_key_auth": {"matched_model_access_groups": ["auth"]}
                }},
                "auth_matched_model_access_groups": ["attribute"]
            }"#
        ),
        vec!["stamped".to_owned()]
    );
}

#[test]
fn the_auth_mapping_supplies_groups_when_nothing_was_stamped() {
    assert_eq!(
        groups(
            r#"{
                "litellm_params": {"metadata": {
                    "user_api_key_auth": {"matched_model_access_groups": ["auth", "auth"]}
                }},
                "auth_matched_model_access_groups": ["attribute"]
            }"#
        ),
        vec!["auth".to_owned()]
    );
}

#[test]
fn the_auth_attribute_supplies_groups_when_the_mapping_carries_none() {
    assert_eq!(
        groups(
            r#"{
                "litellm_params": {"metadata": {"some": "metadata"}},
                "auth_matched_model_access_groups": ["attribute"]
            }"#
        ),
        vec!["attribute".to_owned()]
    );
}

#[test]
fn the_litellm_metadata_mapping_is_the_last_fallback() {
    assert_eq!(
        groups(
            r#"{
                "litellm_params": {
                    "metadata": {},
                    "litellm_metadata": {"user_api_key_matched_model_access_groups": ["litellm"]}
                }
            }"#
        ),
        vec!["litellm".to_owned()]
    );
}

#[test]
fn a_non_mapping_metadata_is_skipped() {
    assert_eq!(
        groups(
            r#"{
                "litellm_params": {
                    "metadata": "not a mapping",
                    "litellm_metadata": {"user_api_key_matched_model_access_groups": ["litellm"]}
                }
            }"#
        ),
        vec!["litellm".to_owned()]
    );
}

#[test]
fn no_sources_name_no_groups() {
    assert!(groups(r#"{"litellm_params": {"metadata": {}, "litellm_metadata": {}}}"#).is_empty());
    assert!(request_model_access_groups(&AccessGroupSources::default()).is_empty());
}

#[test]
fn coercion_keeps_only_nonempty_texts_once() {
    let value = json(r#"["a", "", 5, "b", "a", null, ["a"]]"#);
    assert_eq!(
        coerce_model_access_groups(Some(&value)),
        vec!["a".to_owned(), "b".to_owned()]
    );
    assert!(coerce_model_access_groups(None).is_empty());
    let tuple_as_text = json(r#""a""#);
    assert!(coerce_model_access_groups(Some(&tuple_as_text)).is_empty());
}
