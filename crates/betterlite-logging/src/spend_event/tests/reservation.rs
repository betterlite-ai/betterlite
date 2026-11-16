use betterlite_models::decisions::Json;

use super::support::{built, json, mapping, options};
use crate::spend_event::event::{Mapping, SpendEvent};
use crate::spend_event::input::{CachePresence, TypedAuth, TypedAuths};

const KEY: &str = "user_api_key_budget_reservation";

fn reservation(bucket: Option<&Mapping>) -> Option<Json> {
    bucket.and_then(|entries| entries.get(KEY)).cloned()
}

fn both(event: &SpendEvent) -> [Option<Json>; 2] {
    [
        reservation(event.litellm_params.litellm_metadata.as_ref()),
        reservation(event.litellm_params.metadata.as_ref()),
    ]
}

#[test]
fn litellm_metadata_is_read_before_metadata_and_copied_into_both_buckets() {
    let kwargs = r#"{"litellm_params": {
        "litellm_metadata": {"user_api_key_budget_reservation": {"id": "first"}},
        "metadata": {"user_api_key_budget_reservation": {"id": "second"}}}}"#;
    let event = built(kwargs, &options(false, CachePresence::On));
    let first = Some(json(r#"{"id":"first"}"#));
    assert_eq!(both(&event), [first.clone(), first]);
}

#[test]
fn an_empty_direct_mapping_still_takes_precedence() {
    let kwargs = r#"{"litellm_params": {
        "litellm_metadata": {"user_api_key_budget_reservation": {}},
        "metadata": {"user_api_key_budget_reservation": {"id": "second"}}}}"#;
    let event = built(kwargs, &options(false, CachePresence::On));
    let empty = Some(json("{}"));
    assert_eq!(both(&event), [empty.clone(), empty]);
}

#[test]
fn a_mapping_auth_supplies_the_reservation_when_there_is_no_direct_one() {
    let kwargs = r#"{"litellm_params": {"metadata": {
        "user_api_key_auth": {"budget_reservation": {"id": "from-auth"}}, "keep": 1}}}"#;
    let event = built(kwargs, &options(false, CachePresence::On));
    assert_eq!(both(&event), [None, Some(json(r#"{"id":"from-auth"}"#))]);
    assert_eq!(event.litellm_params.litellm_metadata, None);
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
    assert_eq!(both(&event)[1], Some(json(r#"{"id":"typed"}"#)));
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
    assert_eq!(both(&event), [None, None]);
}

#[test]
fn an_absent_bucket_is_not_manufactured() {
    let kwargs =
        r#"{"litellm_params": {"metadata": {"user_api_key_budget_reservation": {"id": "only"}}}}"#;
    let event = built(kwargs, &options(false, CachePresence::On));
    assert_eq!(event.litellm_params.litellm_metadata, None);
    assert_eq!(both(&event)[1], Some(json(r#"{"id":"only"}"#)));
}

#[test]
fn auth_objects_and_the_otel_span_leave_every_bucket() {
    let kwargs = r#"{"litellm_params": {
        "metadata": {"user_api_key_auth": {"x": 1}, "litellm_parent_otel_span": "span", "team": "t1"},
        "litellm_metadata": {"user_api_key_auth": {"x": 2}, "litellm_parent_otel_span": "span", "org": "o1"}}}"#;
    let event = built(kwargs, &options(false, CachePresence::On));
    let metadata = event.litellm_params.metadata.unwrap();
    let litellm_metadata = event.litellm_params.litellm_metadata.unwrap();
    assert_eq!(metadata.len(), 1);
    assert!(metadata.contains_key("team"));
    assert_eq!(litellm_metadata.len(), 1);
    assert!(litellm_metadata.contains_key("org"));
}
