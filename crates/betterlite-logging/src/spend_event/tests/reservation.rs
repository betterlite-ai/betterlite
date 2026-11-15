use serde_json::{Map, Value};

use super::support::{built, mapping, options};
use crate::spend_event::input::{CachePresence, TypedAuth, TypedAuths};

fn reservation_id(event: &Value, bucket: &str) -> Option<String> {
    event["litellm_params"][bucket]["user_api_key_budget_reservation"]["id"]
        .as_str()
        .map(str::to_owned)
}

#[test]
fn litellm_metadata_is_read_before_metadata_and_copied_into_both_buckets() {
    let kwargs = r#"{"litellm_params": {
        "litellm_metadata": {"user_api_key_budget_reservation": {"id": "first"}},
        "metadata": {"user_api_key_budget_reservation": {"id": "second"}}}}"#;
    let event = built(kwargs, &options(false, CachePresence::On));
    assert_eq!(
        reservation_id(&event, "litellm_metadata").as_deref(),
        Some("first")
    );
    assert_eq!(reservation_id(&event, "metadata").as_deref(), Some("first"));
}

#[test]
fn an_empty_direct_mapping_still_takes_precedence() {
    let kwargs = r#"{"litellm_params": {
        "litellm_metadata": {"user_api_key_budget_reservation": {}},
        "metadata": {"user_api_key_budget_reservation": {"id": "second"}}}}"#;
    let event = built(kwargs, &options(false, CachePresence::On));
    let copied = &event["litellm_params"]["metadata"]["user_api_key_budget_reservation"];
    assert_eq!(copied.as_object().map(Map::len), Some(0));
}

#[test]
fn a_mapping_auth_supplies_the_reservation_when_there_is_no_direct_one() {
    let kwargs = r#"{"litellm_params": {"metadata": {
        "user_api_key_auth": {"budget_reservation": {"id": "from-auth"}}, "keep": 1}}}"#;
    let event = built(kwargs, &options(false, CachePresence::On));
    assert_eq!(
        reservation_id(&event, "metadata").as_deref(),
        Some("from-auth")
    );
    assert_eq!(event["litellm_params"]["litellm_metadata"], Value::Null);
}

#[test]
fn a_typed_auth_beats_a_mapping_auth_in_the_same_bucket() {
    let kwargs = r#"{"litellm_params": {"metadata": {
        "user_api_key_auth": {"budget_reservation": {"id": "mapping"}}}}}"#;
    let mut call = options(false, CachePresence::On);
    call.typed_auth = TypedAuths {
        metadata: Some(TypedAuth {
            budget_reservation: Some(mapping(r#"{"id": "typed"}"#)),
        }),
        litellm_metadata: None,
    };
    let event = built(kwargs, &call);
    assert_eq!(reservation_id(&event, "metadata").as_deref(), Some("typed"));
}

#[test]
fn a_typed_auth_without_a_reservation_does_not_fall_back_to_a_mapping_auth() {
    let kwargs = r#"{"litellm_params": {"metadata": {
        "user_api_key_auth": {"budget_reservation": {"id": "mapping"}}}}}"#;
    let mut call = options(false, CachePresence::On);
    call.typed_auth = TypedAuths {
        metadata: Some(TypedAuth {
            budget_reservation: None,
        }),
        litellm_metadata: None,
    };
    let event = built(kwargs, &call);
    assert_eq!(reservation_id(&event, "metadata"), None);
}

#[test]
fn an_absent_bucket_is_not_manufactured() {
    let kwargs =
        r#"{"litellm_params": {"metadata": {"user_api_key_budget_reservation": {"id": "only"}}}}"#;
    let event = built(kwargs, &options(false, CachePresence::On));
    assert_eq!(event["litellm_params"]["litellm_metadata"], Value::Null);
    assert_eq!(reservation_id(&event, "metadata").as_deref(), Some("only"));
}

#[test]
fn auth_objects_and_the_otel_span_leave_every_bucket() {
    let kwargs = r#"{"litellm_params": {
        "metadata": {"user_api_key_auth": {"x": 1}, "litellm_parent_otel_span": "span", "team": "t1"},
        "litellm_metadata": {"user_api_key_auth": {"x": 2}, "litellm_parent_otel_span": "span", "org": "o1"}}}"#;
    let event = built(kwargs, &options(false, CachePresence::On));
    let metadata = event["litellm_params"]["metadata"].as_object().unwrap();
    let litellm_metadata = event["litellm_params"]["litellm_metadata"]
        .as_object()
        .unwrap();
    assert_eq!(metadata.keys().collect::<Vec<_>>(), ["team"]);
    assert_eq!(litellm_metadata.keys().collect::<Vec<_>>(), ["org"]);
}
