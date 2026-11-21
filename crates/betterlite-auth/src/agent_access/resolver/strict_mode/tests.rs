#![cfg(test)]

use rstest::rstest;

use crate::agent_access::error::AgentAccessError;
use crate::agent_access::resolver::resolve_agent_access;
use crate::agent_access::tests::{
    FakeGrants, FakeHumans, FakeTargets, auth, calls, ids, rejected_status, resolver, restricted,
};
use crate::agent_access_groups::access::AgentAccess;
use crate::identity::{AgentIdentityFailure, FailureCode};

#[tokio::test]
async fn strict_unrestricted_key_and_team_access_denies_before_the_caller_and_ceiling() {
    let mut grants = FakeGrants::default();
    grants.caller = Ok(restricted(&["a"]));
    grants.ceiling = Ok(Some(ids(&["a"])));
    let access_resolver = resolver(grants, FakeTargets::default(), FakeHumans::default());
    let key = auth(r#"{"user_id":"alice"}"#);
    let access = resolve_agent_access(&access_resolver, Some(&key), true)
        .await
        .unwrap();
    assert_eq!(access, restricted(&[]));
    assert_eq!(
        calls(&access_resolver.grants.calls),
        ["key_team alice strict=true"]
    );
}

#[tokio::test]
async fn strict_off_keeps_unrestricted_key_and_team_access_unrestricted() {
    let access_resolver = resolver(
        FakeGrants::default(),
        FakeTargets::default(),
        FakeHumans::default(),
    );
    let key = auth(r#"{"user_id":"alice"}"#);
    let access = resolve_agent_access(&access_resolver, Some(&key), false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Unrestricted);
}

#[tokio::test]
async fn strict_restricted_access_reads_the_caller_and_ceiling_strictly() {
    let mut grants = FakeGrants::default();
    grants
        .key_team
        .insert(String::from("alice"), Ok(restricted(&["a", "b"])));
    grants.caller = Ok(restricted(&["a", "b"]));
    grants.ceiling = Ok(Some(ids(&["a", "c"])));
    let access_resolver = resolver(grants, FakeTargets::default(), FakeHumans::default());
    let key = auth(r#"{"user_id":"alice"}"#);
    let access = resolve_agent_access(&access_resolver, Some(&key), true)
        .await
        .unwrap();
    assert_eq!(access, restricted(&["a"]));
    assert_eq!(
        calls(&access_resolver.grants.calls),
        [
            "key_team alice strict=true",
            "caller alice strict=true",
            "ceiling alice strict=true",
        ]
    );
}

#[derive(Clone, Copy)]
enum FailingPort {
    KeyTeam,
    Caller,
    Ceiling,
}

#[rstest]
#[case::key_and_team(FailingPort::KeyTeam)]
#[case::caller(FailingPort::Caller)]
#[case::ceiling(FailingPort::Ceiling)]
#[tokio::test]
async fn strict_port_errors_propagate(#[case] failing: FailingPort) {
    let mut grants = FakeGrants::default();
    grants
        .key_team
        .insert(String::from("alice"), Ok(restricted(&["a"])));
    match failing {
        FailingPort::KeyTeam => {
            grants.key_team.insert(String::from("alice"), Err(503));
        }
        FailingPort::Caller => grants.caller = Err(503),
        FailingPort::Ceiling => grants.ceiling = Err(503),
    }
    let access_resolver = resolver(grants, FakeTargets::default(), FakeHumans::default());
    let key = auth(r#"{"user_id":"alice"}"#);
    let outcome = resolve_agent_access(&access_resolver, Some(&key), true).await;
    assert_eq!(rejected_status(outcome), 503);
}

#[rstest]
#[case::policy_unavailable(AgentAccessError::PolicyUnavailable, 503)]
#[case::rejected_reload(AgentAccessError::Rejected { status: 401, detail: String::from("expired") }, 401)]
#[case::team_not_granted(AgentAccessError::TeamNotGranted, 403)]
#[case::identity_denied(AgentAccessError::Identity(AgentIdentityFailure { code: FailureCode::IdentityDenied, message: String::from("no") }), 403)]
#[case::identity_policy_unavailable(AgentAccessError::Identity(AgentIdentityFailure { code: FailureCode::PolicyUnavailable, message: String::from("down") }), 503)]
fn strict_errors_answer_with_their_status(#[case] error: AgentAccessError, #[case] status: u16) {
    assert_eq!(error.status(), status);
}

#[test]
fn strict_errors_keep_litellm_messages() {
    assert_eq!(
        AgentAccessError::PolicyUnavailable.to_string(),
        "Agent invocation policy is unavailable"
    );
    assert_eq!(
        AgentAccessError::TeamNotGranted.to_string(),
        "Select a team that grants access to this agent using x-litellm-team-id"
    );
}
