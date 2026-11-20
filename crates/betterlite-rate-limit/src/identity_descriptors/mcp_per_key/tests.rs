#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;

use crate::limiter_records::{RateLimitDescriptor, RateLimitDescriptorRateLimit};

use super::add_mcp_per_key_descriptor;

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

fn descriptor(value: String, requests_per_unit: i64) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: "mcp_per_key".to_owned(),
        value,
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: Some(requests_per_unit),
            tokens_per_unit: None,
            max_parallel_requests: None,
            window_size: Some(60),
        }),
    }
}

fn built(auth: &UserAPIKeyAuth, server: Option<&str>) -> Vec<RateLimitDescriptor> {
    let mut out = Vec::new();
    add_mcp_per_key_descriptor(auth, server, 60, &mut out);
    out
}

#[test]
fn adds_a_descriptor_for_a_configured_server() {
    let auth = keyed(r#"{"mcp_rpm_limit": {"server-a": 7}}"#);
    assert_eq!(
        built(&auth, Some("server-a")),
        vec![descriptor(format!("{API_KEY}:server-a"), 7)]
    );
}

#[test]
fn falls_back_to_the_team_metadata_limit() {
    let auth = auth(&format!(
        r#"{{"api_key": "{API_KEY}", "team_metadata": {{"mcp_rpm_limit": {{"server-a": 3}}}}}}"#
    ));
    assert_eq!(
        built(&auth, Some("server-a")),
        vec![descriptor(format!("{API_KEY}:server-a"), 3)]
    );
}

#[test]
fn adds_nothing_without_a_server_name() {
    let auth = keyed(r#"{"mcp_rpm_limit": {"server-a": 7}}"#);
    assert!(built(&auth, None).is_empty());
}

#[test]
fn adds_nothing_without_an_api_key() {
    let auth = auth(r#"{"metadata": {"mcp_rpm_limit": {"server-a": 7}}}"#);
    assert!(built(&auth, Some("server-a")).is_empty());
}

#[test]
fn adds_nothing_without_a_configured_limit() {
    let auth = keyed("{}");
    assert!(built(&auth, Some("server-a")).is_empty());
}

#[test]
fn skips_a_server_without_a_limit() {
    let auth = keyed(r#"{"mcp_rpm_limit": {"other": 1}}"#);
    assert!(built(&auth, Some("server-a")).is_empty());
}

#[test]
fn keeps_a_zero_limit() {
    let auth = keyed(r#"{"mcp_rpm_limit": {"server-a": 0}}"#);
    assert_eq!(
        built(&auth, Some("server-a")),
        vec![descriptor(format!("{API_KEY}:server-a"), 0)]
    );
}

#[test]
fn appends_after_existing_descriptors() {
    let auth = keyed(r#"{"mcp_rpm_limit": {"server-a": 7}}"#);
    let existing = descriptor("other:key".to_owned(), 9);
    let mut out = vec![existing.clone()];
    add_mcp_per_key_descriptor(&auth, Some("server-a"), 60, &mut out);
    assert_eq!(
        out,
        vec![existing, descriptor(format!("{API_KEY}:server-a"), 7)]
    );
}
