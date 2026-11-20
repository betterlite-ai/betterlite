#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_core::api::request_destination::RequestData;
use betterlite_db::budget_permission_rows::JsonData;

use crate::limiter_records::{RateLimitDescriptor, RateLimitDescriptorRateLimit};

use super::add_tag_per_key_descriptor;

const API_KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn auth(wire: &str) -> UserAPIKeyAuth {
    let values: BTreeMap<String, JsonData> = serde_json::from_str(wire).expect("auth fields");
    UserAPIKeyAuth::construct(AuthInput::Dict(values)).expect("the auth fixture builds")
}

fn keyed(metadata: &str) -> UserAPIKeyAuth {
    auth(&format!(
        r#"{{"api_key": "{API_KEY}", "metadata": {metadata}}}"#
    ))
}

fn data(wire: &str) -> RequestData {
    serde_json::from_str(wire).expect("the request body fixture decodes")
}

fn descriptor(value: String, requests_per_unit: i64) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: "tag_per_key".to_owned(),
        value,
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: Some(requests_per_unit),
            tokens_per_unit: None,
            max_parallel_requests: None,
            window_size: Some(60),
        }),
    }
}

fn built(auth: &UserAPIKeyAuth, wire: &str) -> Vec<RateLimitDescriptor> {
    let mut out = Vec::new();
    add_tag_per_key_descriptor(auth, &data(wire), 60, &mut out);
    out
}

#[test]
fn adds_one_descriptor_per_configured_tag() {
    let auth = keyed(r#"{"tag_rpm_limit": {"tag-a": 10, "tag-b": 5}}"#);
    assert_eq!(
        built(&auth, r#"{"tags": ["tag-a", "tag-b"]}"#),
        vec![
            descriptor(format!("{API_KEY}:tag-a"), 10),
            descriptor(format!("{API_KEY}:tag-b"), 5),
        ]
    );
}

#[test]
fn keys_the_descriptor_by_the_hashed_api_key() {
    let auth = keyed(r#"{"tag_rpm_limit": {"tag-a": 10}}"#);
    let hashed = account_limits(&auth).hashed_api_key.expect("the key hash");
    let descriptors = built(&auth, r#"{"tags": ["tag-a"]}"#);
    assert_eq!(descriptors[0].value, format!("{hashed}:tag-a"));
}

#[test]
fn adds_nothing_without_an_api_key() {
    let auth = auth(r#"{"metadata": {"tag_rpm_limit": {"tag-a": 10}}}"#);
    assert!(built(&auth, r#"{"tags": ["tag-a"]}"#).is_empty());
}

#[test]
fn adds_nothing_without_key_tag_limits() {
    let auth = keyed("{}");
    assert!(built(&auth, r#"{"tags": ["tag-a"]}"#).is_empty());
}

#[test]
fn skips_tags_without_a_limit() {
    let auth = keyed(r#"{"tag_rpm_limit": {"other": 5}}"#);
    assert!(built(&auth, r#"{"tags": ["tag-a"]}"#).is_empty());
}

#[test]
fn keeps_the_mapped_tags_when_another_is_unmapped() {
    let auth = keyed(r#"{"tag_rpm_limit": {"tag-a": 1}}"#);
    assert_eq!(
        built(&auth, r#"{"tags": ["tag-a", "tag-b"]}"#),
        vec![descriptor(format!("{API_KEY}:tag-a"), 1)]
    );
}

#[test]
fn deduplicates_a_repeated_tag() {
    let auth = keyed(r#"{"tag_rpm_limit": {"tag-a": 1}}"#);
    assert_eq!(
        built(&auth, r#"{"tags": ["tag-a", "tag-a"]}"#),
        vec![descriptor(format!("{API_KEY}:tag-a"), 1)]
    );
}

#[test]
fn follows_the_request_tag_order() {
    let auth = keyed(r#"{"tag_rpm_limit": {"tag-a": 1, "tag-b": 2}}"#);
    let descriptors = built(&auth, r#"{"tags": ["tag-b", "tag-a"]}"#);
    assert_eq!(descriptors[0].value, format!("{API_KEY}:tag-b"));
    assert_eq!(descriptors[1].value, format!("{API_KEY}:tag-a"));
}

#[test]
fn keeps_a_zero_limit() {
    let auth = keyed(r#"{"tag_rpm_limit": {"tag-a": 0}}"#);
    assert_eq!(
        built(&auth, r#"{"tags": ["tag-a"]}"#),
        vec![descriptor(format!("{API_KEY}:tag-a"), 0)]
    );
}

#[test]
fn appends_after_existing_descriptors() {
    let auth = keyed(r#"{"tag_rpm_limit": {"tag-a": 1}}"#);
    let existing = descriptor("other:key".to_owned(), 9);
    let mut out = vec![existing.clone()];
    add_tag_per_key_descriptor(&auth, &data(r#"{"tags": ["tag-a"]}"#), 60, &mut out);
    assert_eq!(
        out,
        vec![existing, descriptor(format!("{API_KEY}:tag-a"), 1)]
    );
}
