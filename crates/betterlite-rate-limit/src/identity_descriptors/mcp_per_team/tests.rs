#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::auth_limit_inputs::key_limits::ModelLimits;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;

use crate::limiter_records::{RateLimitDescriptor, RateLimitDescriptorRateLimit};

use super::{SourceTeamLimits, add_mcp_per_team_descriptor};

const API_KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn auth(wire: &str) -> UserAPIKeyAuth {
    let values: BTreeMap<String, JsonData> = serde_json::from_str(wire).expect("auth fields");
    UserAPIKeyAuth::construct(AuthInput::Dict(values)).expect("the auth fixture builds")
}

fn team_keyed(team_id: &str, team_metadata: &str) -> UserAPIKeyAuth {
    auth(&format!(
        r#"{{"api_key": "{API_KEY}", "team_id": "{team_id}", "team_metadata": {team_metadata}}}"#
    ))
}

fn source(entries: &[(&str, &[(&str, i64)])]) -> SourceTeamLimits {
    entries
        .iter()
        .map(|(team_id, servers)| {
            let limits: ModelLimits = servers
                .iter()
                .map(|(server, limit)| ((*server).to_owned(), *limit))
                .collect();
            ((*team_id).to_owned(), limits)
        })
        .collect()
}

fn descriptor(value: String, requests_per_unit: i64) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: "mcp_per_team".to_owned(),
        value,
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: Some(requests_per_unit),
            tokens_per_unit: None,
            max_parallel_requests: None,
            window_size: Some(60),
        }),
    }
}

fn built(
    auth: &UserAPIKeyAuth,
    server: Option<&str>,
    source_teams: Option<&SourceTeamLimits>,
) -> Vec<RateLimitDescriptor> {
    let mut out = Vec::new();
    add_mcp_per_team_descriptor(auth, server, source_teams, 60, &mut out);
    out
}

#[test]
fn charges_the_key_team() {
    let auth = team_keyed("team-a", r#"{"mcp_rpm_limit": {"server-a": 5}}"#);
    assert_eq!(
        built(&auth, Some("server-a"), None),
        vec![descriptor("team-a:server-a".to_owned(), 5)]
    );
}

#[test]
fn charges_every_source_team() {
    let auth = auth(&format!(r#"{{"api_key": "{API_KEY}"}}"#));
    let teams = source(&[
        ("team-b", &[("server-a", 2)]),
        ("team-c", &[("server-a", 3)]),
    ]);
    assert_eq!(
        built(&auth, Some("server-a"), Some(&teams)),
        vec![
            descriptor("team-b:server-a".to_owned(), 2),
            descriptor("team-c:server-a".to_owned(), 3),
        ]
    );
}

#[test]
fn charges_the_key_team_before_source_teams() {
    let auth = team_keyed("team-a", r#"{"mcp_rpm_limit": {"server-a": 2}}"#);
    let teams = source(&[("team-b", &[("server-a", 3)])]);
    assert_eq!(
        built(&auth, Some("server-a"), Some(&teams)),
        vec![
            descriptor("team-a:server-a".to_owned(), 2),
            descriptor("team-b:server-a".to_owned(), 3),
        ]
    );
}

#[test]
fn charges_a_team_listed_in_both_sources_twice() {
    let auth = team_keyed("team-a", r#"{"mcp_rpm_limit": {"server-a": 2}}"#);
    let teams = source(&[("team-a", &[("server-a", 4)])]);
    assert_eq!(
        built(&auth, Some("server-a"), Some(&teams)),
        vec![
            descriptor("team-a:server-a".to_owned(), 2),
            descriptor("team-a:server-a".to_owned(), 4),
        ]
    );
}

#[test]
fn skips_source_teams_without_the_server() {
    let auth = auth(&format!(r#"{{"api_key": "{API_KEY}"}}"#));
    let teams = source(&[("team-b", &[("other", 9)])]);
    assert!(built(&auth, Some("server-a"), Some(&teams)).is_empty());
}

#[test]
fn skips_the_key_team_without_a_configured_limit() {
    let auth = team_keyed("team-a", "{}");
    assert!(built(&auth, Some("server-a"), None).is_empty());
}

#[test]
fn adds_nothing_without_a_server_name() {
    let auth = team_keyed("team-a", r#"{"mcp_rpm_limit": {"server-a": 5}}"#);
    assert!(built(&auth, None, None).is_empty());
}

#[test]
fn adds_nothing_without_a_team_or_a_source() {
    let auth = auth(&format!(r#"{{"api_key": "{API_KEY}"}}"#));
    assert!(built(&auth, Some("server-a"), None).is_empty());
}

#[test]
fn keeps_a_zero_limit() {
    let auth = team_keyed("team-a", r#"{"mcp_rpm_limit": {"server-a": 0}}"#);
    assert_eq!(
        built(&auth, Some("server-a"), None),
        vec![descriptor("team-a:server-a".to_owned(), 0)]
    );
}

#[test]
fn appends_after_existing_descriptors() {
    let auth = team_keyed("team-a", r#"{"mcp_rpm_limit": {"server-a": 5}}"#);
    let existing = descriptor("other:key".to_owned(), 9);
    let mut out = vec![existing.clone()];
    add_mcp_per_team_descriptor(&auth, Some("server-a"), None, 60, &mut out);
    assert_eq!(
        out,
        vec![existing, descriptor("team-a:server-a".to_owned(), 5)]
    );
}
