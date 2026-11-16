#![cfg(test)]

use crate::reconciliation_operations::scope_targets::{ScopeMetadata, collect_tpm_scope_targets};
use crate::reconciliation_operations::tests::support::{pairs, tags};

fn full_metadata() -> ScopeMetadata {
    ScopeMetadata {
        user_api_key_hash: Some("key-1".to_owned()),
        user_api_key_user_id: Some("user-1".to_owned()),
        user_api_key_team_id: Some("team-1".to_owned()),
        user_api_key_org_id: Some("org-1".to_owned()),
        user_api_key_project_id: Some("proj-1".to_owned()),
        user_api_key_end_user_id: Some("end-1".to_owned()),
        agent_id: Some("agent-1".to_owned()),
        session_id: Some("sess-1".to_owned()),
        trace_id: Some("trace-1".to_owned()),
    }
}

#[test]
fn every_scope_is_enumerated_in_pipeline_order() {
    assert_eq!(
        collect_tpm_scope_targets(
            &full_metadata(),
            None,
            Some("gpt-4"),
            &tags(&["zeta", "alpha"])
        ),
        pairs(&[
            ("api_key", "key-1"),
            ("user", "user-1"),
            ("team", "team-1"),
            ("team_member", "team-1:user-1"),
            ("end_user", "end-1"),
            ("organization", "org-1"),
            ("model_per_key", "key-1:gpt-4"),
            ("model_per_team", "team-1:gpt-4"),
            ("model_per_organization", "org-1:gpt-4"),
            ("model_per_project", "proj-1:gpt-4"),
            ("agent", "agent-1"),
            ("agent_session", "agent-1:sess-1"),
            ("tag", "alpha"),
            ("tag", "zeta"),
        ])
    );
}

#[test]
fn without_a_model_group_the_model_scopes_are_skipped() {
    let metadata = ScopeMetadata {
        user_api_key_hash: Some("key-1".to_owned()),
        user_api_key_team_id: Some("team-1".to_owned()),
        user_api_key_project_id: Some("proj-1".to_owned()),
        user_api_key_user_id: None,
        user_api_key_org_id: None,
        user_api_key_end_user_id: None,
        agent_id: None,
        session_id: None,
        trace_id: None,
    };
    assert_eq!(
        collect_tpm_scope_targets(&metadata, None, None, &tags(&[])),
        pairs(&[("api_key", "key-1"), ("team", "team-1")])
    );
}

#[test]
fn tags_are_sorted_after_the_identity_scopes() {
    assert_eq!(
        collect_tpm_scope_targets(
            &ScopeMetadata::default(),
            None,
            None,
            &tags(&["zeta", "alpha", "mike"])
        ),
        pairs(&[("tag", "alpha"), ("tag", "mike"), ("tag", "zeta")])
    );
}
