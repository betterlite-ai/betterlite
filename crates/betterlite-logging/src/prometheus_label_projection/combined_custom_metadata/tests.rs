#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::combined_custom_metadata;
use crate::fields::payload::records::StandardLoggingMetadata;

fn metadata(encoded: &str) -> StandardLoggingMetadata {
    serde_json::from_str(encoded).expect("the standard logging metadata decodes")
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[test]
fn absent_metadata_has_no_custom_labels() {
    let combined = combined_custom_metadata(None).expect("the combined metadata");
    assert!(combined.is_empty());
}

#[test]
fn default_metadata_has_no_custom_labels() {
    let combined = combined_custom_metadata(Some(&StandardLoggingMetadata::default()))
        .expect("the combined metadata");
    assert!(combined.is_empty());
}

#[test]
fn top_level_scalars_and_subobject_entries_are_combined() {
    let metadata = metadata(
        r#"{
            "user_api_key_team_alias": "team-a",
            "user_api_key_user_spend": 1.5,
            "applied_guardrails": ["pii"],
            "user_api_key_project_alias": null,
            "requester_metadata": {"source": "sdk", "nested": {"zone": "z1"}},
            "user_api_key_auth_metadata": {"auth_source": "jwt"},
            "spend_logs_metadata": {"log_bucket": "b1"},
            "usage_object": {"cost": "1"}
        }"#,
    );
    let combined = combined_custom_metadata(Some(&metadata)).expect("the combined metadata");
    assert_eq!(
        combined.get("user_api_key_team_alias"),
        Some(&text("team-a"))
    );
    assert!(matches!(
        combined.get("user_api_key_user_spend"),
        Some(Json::Number(_))
    ));
    assert_eq!(
        combined.get("applied_guardrails"),
        Some(&Json::List(Vec::from([text("pii")])))
    );
    assert!(!combined.contains_key("user_api_key_project_alias"));
    assert!(!combined.contains_key("requester_metadata"));
    assert!(!combined.contains_key("usage_object"));
    assert!(!combined.contains_key("cost"));
    assert_eq!(combined.get("source"), Some(&text("sdk")));
    assert_eq!(combined.get("auth_source"), Some(&text("jwt")));
    assert_eq!(combined.get("log_bucket"), Some(&text("b1")));
    assert_eq!(
        combined.get("nested"),
        Some(&Json::Object(IndexMap::from([(
            "zone".to_owned(),
            text("z1")
        )])))
    );
}

#[test]
fn later_sources_override_earlier_ones() {
    let metadata = metadata(
        r#"{
            "user_api_key_team_alias": "top",
            "requester_metadata": {"user_api_key_team_alias": "requester", "region": "requester"},
            "user_api_key_auth_metadata": {"region": "auth"},
            "spend_logs_metadata": {"region": "spend"}
        }"#,
    );
    let combined = combined_custom_metadata(Some(&metadata)).expect("the combined metadata");
    assert_eq!(
        combined.get("user_api_key_team_alias"),
        Some(&text("requester"))
    );
    assert_eq!(combined.get("region"), Some(&text("spend")));
}

#[test]
fn a_subobject_that_is_not_an_object_stays_a_scalar() {
    let metadata = metadata(r#"{"requester_metadata": "plain", "spend_logs_metadata": ["a"]}"#);
    let combined = combined_custom_metadata(Some(&metadata)).expect("the combined metadata");
    assert_eq!(combined.get("requester_metadata"), Some(&text("plain")));
    assert_eq!(
        combined.get("spend_logs_metadata"),
        Some(&Json::List(Vec::from([text("a")])))
    );
}
