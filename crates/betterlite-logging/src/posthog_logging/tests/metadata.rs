use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::posthog_logging::event::{PostHogEventError, create_event_payload};
use crate::posthog_logging::tests::support::{fixed_id, number, object, payload, text};

const INTERNAL_FIELDS: [&str; 32] = [
    "endpoint",
    "caching_groups",
    "user_api_key_hash",
    "user_api_key_alias",
    "user_api_key_team_id",
    "user_api_key_user_id",
    "user_api_key_org_id",
    "user_api_key_team_alias",
    "user_api_key_end_user_id",
    "user_api_key_user_email",
    "user_api_key",
    "user_api_end_user_max_budget",
    "litellm_api_version",
    "global_max_parallel_requests",
    "user_api_key_team_max_budget",
    "user_api_key_team_spend",
    "user_api_key_spend",
    "user_api_key_max_budget",
    "user_api_key_model_max_budget",
    "user_api_key_metadata",
    "headers",
    "litellm_parent_otel_span",
    "requester_ip_address",
    "model_group",
    "model_group_size",
    "deployment",
    "model_info",
    "api_base",
    "hidden_params",
    "parent_run_id",
    "parent_id",
    "user_id",
];

fn properties(metadata: Json) -> IndexMap<String, Json> {
    let mut payload = payload();
    payload.metadata.requester_metadata = Some(metadata);
    create_event_payload(&payload, fixed_id)
        .expect("the event builds")
        .properties
}

#[test]
fn posthog_metadata_adds_custom_keys() {
    let properties = properties(object(&[
        ("team", text("analytics")),
        ("attempt", number(2)),
        ("flagged", Json::Bool(true)),
    ]));
    assert_eq!(properties.get("team"), Some(&text("analytics")));
    assert_eq!(properties.get("attempt"), Some(&number(2)));
    assert_eq!(properties.get("flagged"), Some(&Json::Bool(true)));
}

#[test]
fn posthog_metadata_drops_every_internal_field() {
    let fields: IndexMap<String, Json> = INTERNAL_FIELDS
        .iter()
        .map(|field| ((*field).to_owned(), text("internal")))
        .collect();
    let properties = properties(Json::Object(fields));
    let leaked: Vec<&String> = properties
        .keys()
        .filter(|key| INTERNAL_FIELDS.contains(&key.as_str()))
        .collect();
    assert!(leaked.is_empty(), "internal fields leaked: {leaked:?}");
    assert_eq!(properties.get("$ai_parent_id"), Some(&text("internal")));
}

#[test]
fn posthog_metadata_overwrites_a_builtin_property() {
    let properties = properties(object(&[("$ai_model", text("custom-model"))]));
    assert_eq!(properties.get("$ai_model"), Some(&text("custom-model")));
}

#[test]
fn posthog_metadata_rejects_a_non_object_value() {
    let mut payload = payload();
    payload.metadata.requester_metadata = Some(text("not an object"));
    let error = create_event_payload(&payload, fixed_id).expect_err("the metadata type is wrong");
    assert!(matches!(error, PostHogEventError::MetadataType));
}

#[test]
fn posthog_metadata_keeps_the_payload_without_requester_metadata() {
    let properties = properties(Json::Null);
    assert_eq!(properties.get("$ai_model"), Some(&text("gpt-4o")));
}
