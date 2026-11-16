#![cfg(test)]

use crate::reconciliation_operations::scope_targets::{ScopeMetadata, collect_tpm_scope_targets};
use crate::reconciliation_operations::tests::support::{model_scope_metadata, pairs, tags};

#[test]
fn team_member_joins_the_team_and_user_with_a_colon() {
    let metadata = ScopeMetadata {
        user_api_key_user_id: Some("user-1".to_owned()),
        user_api_key_team_id: Some("team-1".to_owned()),
        user_api_key_hash: None,
        user_api_key_org_id: None,
        user_api_key_project_id: None,
        user_api_key_end_user_id: None,
        agent_id: None,
        session_id: None,
        trace_id: None,
    };
    assert_eq!(
        collect_tpm_scope_targets(&metadata, None, None, &tags(&[])),
        pairs(&[
            ("user", "user-1"),
            ("team", "team-1"),
            ("team_member", "team-1:user-1"),
        ])
    );
}

#[test]
fn agent_session_joins_the_agent_and_session_with_a_colon() {
    let metadata = ScopeMetadata {
        agent_id: Some("agent-1".to_owned()),
        session_id: Some("sess-1".to_owned()),
        user_api_key_hash: None,
        user_api_key_user_id: None,
        user_api_key_team_id: None,
        user_api_key_org_id: None,
        user_api_key_project_id: None,
        user_api_key_end_user_id: None,
        trace_id: None,
    };
    assert_eq!(
        collect_tpm_scope_targets(&metadata, None, None, &tags(&[])),
        pairs(&[("agent", "agent-1"), ("agent_session", "agent-1:sess-1"),])
    );
}

#[test]
fn agent_session_falls_back_to_the_trace_id() {
    let metadata = ScopeMetadata {
        agent_id: Some("agent-1".to_owned()),
        trace_id: Some("trace-1".to_owned()),
        user_api_key_hash: None,
        user_api_key_user_id: None,
        user_api_key_team_id: None,
        user_api_key_org_id: None,
        user_api_key_project_id: None,
        user_api_key_end_user_id: None,
        session_id: None,
    };
    assert_eq!(
        collect_tpm_scope_targets(&metadata, None, None, &tags(&[])),
        pairs(&[("agent", "agent-1"), ("agent_session", "agent-1:trace-1"),])
    );
}

#[test]
fn model_scopes_join_the_id_and_the_group() {
    assert_eq!(
        collect_tpm_scope_targets(&model_scope_metadata(), None, Some("gpt-4"), &tags(&[])),
        pairs(&[
            ("api_key", "key-1"),
            ("team", "team-1"),
            ("organization", "org-1"),
            ("model_per_key", "key-1:gpt-4"),
            ("model_per_team", "team-1:gpt-4"),
            ("model_per_organization", "org-1:gpt-4"),
            ("model_per_project", "proj-1:gpt-4"),
        ])
    );
}
