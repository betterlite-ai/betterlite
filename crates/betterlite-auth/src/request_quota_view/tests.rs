#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use super::{RequestQuotaViewExt as _, ServerOnlyFields};
use crate::agent_policy::AgentResponse;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

fn auth(fields: &str) -> UserAPIKeyAuth {
    let values: BTreeMap<String, JsonData> = serde_json::from_str(fields).expect("auth fields");
    UserAPIKeyAuth::construct(AuthInput::Dict(values)).expect("the auth fixture builds")
}

fn policy(agent_id: &str) -> AgentResponse {
    let wire = format!(
        r#"{{"agent_id": "{agent_id}", "agent_name": "{agent_id}", "agent_card_params": {{}}, "rpm_limit": 5}}"#
    );
    serde_json::from_str(&wire).expect("the policy fixture decodes")
}

#[test]
fn copies_the_key_limits_and_the_throttle_percentage() {
    let auth = auth(
        r#"{"rpm_limit": 10, "tpm_limit": 20, "max_parallel_requests": 3,
            "budget_throttle_pct": 0.5}"#,
    );
    let view = auth.request_quota_view();
    assert_eq!(view.rpm_limit, Some(10));
    assert_eq!(view.tpm_limit, Some(20));
    assert_eq!(view.max_parallel_requests, Some(3));
    assert_eq!(view.throttle_pct, Some(0.5_f64));
}

#[test]
fn copies_the_identity_ids() {
    let auth = auth(
        r#"{"user_id": "user-1", "team_id": "team-1", "end_user_id": "end-1",
            "agent_id": "agent-1"}"#,
    );
    let view = auth.request_quota_view();
    assert_eq!(view.user_id, Some("user-1"));
    assert_eq!(view.end_user_id, Some("end-1"));
    assert_eq!(view.agent_id, Some("agent-1"));
}

#[test]
fn copies_the_user_team_team_member_and_end_user_limits() {
    let auth = auth(
        r#"{"user_rpm_limit": 1, "user_tpm_limit": 2, "team_rpm_limit": 3,
            "team_tpm_limit": 4, "team_member_rpm_limit": 5, "team_member_tpm_limit": 6,
            "end_user_rpm_limit": 7, "end_user_tpm_limit": 8}"#,
    );
    let view = auth.request_quota_view();
    assert_eq!(view.user_rpm_limit, Some(1));
    assert_eq!(view.user_tpm_limit, Some(2));
    assert_eq!(view.team_rpm_limit, Some(3));
    assert_eq!(view.team_tpm_limit, Some(4));
    assert_eq!(view.team_member_rpm_limit, Some(5));
    assert_eq!(view.team_member_tpm_limit, Some(6));
    assert_eq!(view.end_user_rpm_limit, Some(7));
    assert_eq!(view.end_user_tpm_limit, Some(8));
}

#[test]
fn an_absent_throttle_percentage_stays_unset() {
    let auth = auth("{}");
    let view = auth.request_quota_view();
    assert_eq!(view.throttle_pct, None);
}

#[test]
fn reads_the_limit_types_from_key_metadata() {
    let auth = auth(
        r#"{"metadata": {"rpm_limit_type": "dynamic", "tpm_limit_type": "guaranteed_throughput"}}"#,
    );
    let view = auth.request_quota_view();
    assert_eq!(view.rpm_limit_type, Some("dynamic"));
    assert_eq!(view.tpm_limit_type, Some("guaranteed_throughput"));
}

#[rstest]
#[case::number("5")]
#[case::boolean("true")]
#[case::null("null")]
#[case::array(r#"["dynamic"]"#)]
#[case::object(r#"{"type": "dynamic"}"#)]
fn a_limit_type_that_is_not_a_string_reads_as_none(#[case] value: &str) {
    let auth = auth(&format!(
        r#"{{"metadata": {{"rpm_limit_type": {value}, "tpm_limit_type": {value}}}}}"#
    ));
    let view = auth.request_quota_view();
    assert_eq!(view.rpm_limit_type, None);
    assert_eq!(view.tpm_limit_type, None);
}

#[test]
fn a_key_without_metadata_has_no_limit_types() {
    let auth = auth("{}");
    let view = auth.request_quota_view();
    assert_eq!(view.rpm_limit_type, None);
    assert_eq!(view.tpm_limit_type, None);
}

#[test]
fn an_auth_without_attached_server_fields_reads_them_as_none() {
    let auth = auth(r#"{"agent_id": "key-agent"}"#);
    let view = auth.request_quota_view();
    assert_eq!(view.invoked_agent_id, None);
    assert!(view.managed_agent_policy.is_none());
    assert!(view.invoked_agent_policy.is_none());
    assert!(view.source_team_rpm_limits.is_none());
}

#[test]
fn returns_the_attached_server_only_fields() {
    let mut auth = auth(r#"{"agent_id": "key-agent"}"#);
    let teams = BTreeMap::from([(
        "team-2".to_owned(),
        BTreeMap::from([("server".to_owned(), 9_i64)]),
    )]);
    auth.attach_server_only_fields(ServerOnlyFields {
        managed_agent_policy: Some(policy("key-agent")),
        invoked_agent_id: Some("other-agent".to_owned()),
        invoked_agent_policy: Some(policy("other-agent")),
        source_team_rpm_limits: Some(teams),
    });
    let view = auth.request_quota_view();
    assert_eq!(view.agent_id, Some("key-agent"));
    assert_eq!(view.invoked_agent_id, Some("other-agent"));
    assert_eq!(
        view.managed_agent_policy
            .map(|policy| policy.agent_id.as_str()),
        Some("key-agent")
    );
    assert_eq!(
        view.invoked_agent_policy
            .map(|policy| policy.agent_id.as_str()),
        Some("other-agent")
    );
    assert_eq!(
        view.source_team_rpm_limits
            .and_then(|by_team| by_team.get("team-2"))
            .and_then(|limits| limits.get("server")),
        Some(&9)
    );
}
