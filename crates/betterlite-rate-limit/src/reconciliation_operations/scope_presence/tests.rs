#![cfg(test)]

use crate::reconciliation_operations::scope_targets::{ScopeMetadata, collect_tpm_scope_targets};
use crate::reconciliation_operations::tests::support::{pairs, tags};

#[test]
fn an_empty_metadata_value_is_omitted() {
    let metadata = ScopeMetadata {
        user_api_key_hash: Some(String::new()),
        user_api_key_user_id: Some(String::new()),
        user_api_key_team_id: Some(String::new()),
        user_api_key_org_id: Some(String::new()),
        user_api_key_project_id: Some(String::new()),
        user_api_key_end_user_id: Some(String::new()),
        agent_id: Some(String::new()),
        session_id: Some("sess-1".to_owned()),
        trace_id: Some(String::new()),
    };
    assert_eq!(
        collect_tpm_scope_targets(&metadata, None, Some("gpt-4"), &tags(&[])),
        Vec::new()
    );
}

#[test]
fn a_blank_value_counts_as_present() {
    let metadata = ScopeMetadata {
        user_api_key_hash: Some(" ".to_owned()),
        user_api_key_user_id: None,
        user_api_key_team_id: None,
        user_api_key_org_id: None,
        user_api_key_project_id: None,
        user_api_key_end_user_id: None,
        agent_id: None,
        session_id: None,
        trace_id: None,
    };
    assert_eq!(
        collect_tpm_scope_targets(&metadata, None, None, &tags(&[])),
        pairs(&[("api_key", " ")])
    );
}

#[test]
fn missing_values_are_omitted() {
    assert_eq!(
        collect_tpm_scope_targets(&ScopeMetadata::default(), None, None, &tags(&[])),
        Vec::new()
    );
}

#[test]
fn the_request_user_replaces_the_metadata_end_user() {
    let metadata = ScopeMetadata {
        user_api_key_end_user_id: Some("logged".to_owned()),
        user_api_key_hash: None,
        user_api_key_user_id: None,
        user_api_key_team_id: None,
        user_api_key_org_id: None,
        user_api_key_project_id: None,
        agent_id: None,
        session_id: None,
        trace_id: None,
    };
    assert_eq!(
        collect_tpm_scope_targets(&metadata, Some("caller"), None, &tags(&[])),
        pairs(&[("end_user", "caller")])
    );
}

#[test]
fn an_empty_request_user_falls_back_to_the_metadata_end_user() {
    let metadata = ScopeMetadata {
        user_api_key_end_user_id: Some("logged".to_owned()),
        user_api_key_hash: None,
        user_api_key_user_id: None,
        user_api_key_team_id: None,
        user_api_key_org_id: None,
        user_api_key_project_id: None,
        agent_id: None,
        session_id: None,
        trace_id: None,
    };
    assert_eq!(
        collect_tpm_scope_targets(&metadata, Some(""), None, &tags(&[])),
        pairs(&[("end_user", "logged")])
    );
}

#[test]
fn team_member_needs_both_ids() {
    let team_only = ScopeMetadata {
        user_api_key_team_id: Some("team-1".to_owned()),
        user_api_key_hash: None,
        user_api_key_user_id: None,
        user_api_key_org_id: None,
        user_api_key_project_id: None,
        user_api_key_end_user_id: None,
        agent_id: None,
        session_id: None,
        trace_id: None,
    };
    assert_eq!(
        collect_tpm_scope_targets(&team_only, None, None, &tags(&[])),
        pairs(&[("team", "team-1")])
    );
    let user_only = ScopeMetadata {
        user_api_key_user_id: Some("user-1".to_owned()),
        user_api_key_hash: None,
        user_api_key_team_id: None,
        user_api_key_org_id: None,
        user_api_key_project_id: None,
        user_api_key_end_user_id: None,
        agent_id: None,
        session_id: None,
        trace_id: None,
    };
    assert_eq!(
        collect_tpm_scope_targets(&user_only, None, None, &tags(&[])),
        pairs(&[("user", "user-1")])
    );
}

#[test]
fn a_session_without_an_agent_adds_nothing() {
    let metadata = ScopeMetadata {
        session_id: Some("sess-1".to_owned()),
        trace_id: Some("trace-1".to_owned()),
        user_api_key_hash: None,
        user_api_key_user_id: None,
        user_api_key_team_id: None,
        user_api_key_org_id: None,
        user_api_key_project_id: None,
        user_api_key_end_user_id: None,
        agent_id: None,
    };
    assert_eq!(
        collect_tpm_scope_targets(&metadata, None, None, &tags(&[])),
        Vec::new()
    );
}
