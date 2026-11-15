#![cfg(test)]

use betterlite_db::postgres_store::session::PostgresStore;
use rstest::rstest;

use crate::agent_identity::error::POLICY_UNAVAILABLE_STATUS;
use crate::agent_policy::{ManagedAgentMode, PolicyFlag};
use crate::agent_store::tests::support::{
    FakeCall, FakeRecords, RecordedSubjectKey, application_claims, autonomous_agent, binding,
    delegated_agent, delegated_claims, human_row, store,
};
use crate::identity::FailureCode;

#[rstest]
#[case::issuer("iss")]
#[case::tenant("tid")]
#[case::client("azp")]
#[tokio::test]
async fn resolve_claim_without_a_required_name_is_unbound(#[case] name: &str) {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.identity = Some(binding("agent-1"));
        state.agent = Some(autonomous_agent());
    }
    let mut claims = application_claims();
    claims.0.remove(name);
    let context = store(&fake)
        .resolve_verified_claims(&claims)
        .await
        .expect("the claim resolution");
    assert!(context.is_none());
    assert!(fake.state().identity_reads.is_empty());
}

#[tokio::test]
async fn resolve_claim_without_the_client_id_is_unbound() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.identity = Some(binding("agent-1"));
        state.agent = Some(autonomous_agent());
    }
    let mut claims = application_claims();
    claims.0.remove("azp");
    let context = store(&fake)
        .resolve_verified_claims(&claims)
        .await
        .expect("the claim resolution");
    assert!(context.is_none());
    assert!(fake.state().identity_reads.is_empty());
}

#[tokio::test]
async fn resolve_unbound_client_is_unbound() {
    let fake = FakeRecords::shared();
    let context = store(&fake)
        .resolve_verified_claims(&application_claims())
        .await
        .expect("the claim resolution");
    assert!(context.is_none());
    assert!(fake.state().agent_reads.is_empty());
}

#[tokio::test]
async fn resolve_missing_agent_is_denied() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.identity = Some(binding("agent-1"));
    }
    let failure = store(&fake)
        .resolve_verified_claims(&application_claims())
        .await
        .expect_err("a missing agent");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "Agent is disabled or no longer bound to an identity"
    );
}

#[tokio::test]
async fn resolve_agent_read_failure_is_policy_unavailable() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.identity = Some(binding("agent-1"));
        state.failure = Some(FakeCall::Agent);
    }
    let failure = store(&fake)
        .resolve_verified_claims(&application_claims())
        .await
        .expect_err("an agent read failure");
    assert_eq!(failure.code, FailureCode::PolicyUnavailable);
    assert_eq!(failure.message, "Agent policy could not be loaded");
}

#[tokio::test]
async fn resolve_disabled_agent_is_denied() {
    let fake = FakeRecords::shared();
    {
        let mut agent = autonomous_agent();
        agent.enabled = PolicyFlag(false);
        let mut state = fake.state();
        state.agent = Some(agent);
        state.identity = Some(binding("agent-1"));
    }
    let failure = store(&fake)
        .resolve_verified_claims(&application_claims())
        .await
        .expect_err("a disabled agent");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "Agent is disabled or no longer bound to an identity"
    );
}

#[tokio::test]
async fn resolve_agent_not_identity_managed_is_denied() {
    let fake = FakeRecords::shared();
    {
        let mut agent = autonomous_agent();
        agent.identity_managed = false;
        let mut state = fake.state();
        state.agent = Some(agent);
        state.identity = Some(binding("agent-1"));
    }
    let failure = store(&fake)
        .resolve_verified_claims(&application_claims())
        .await
        .expect_err("an unmanaged agent");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "Agent is disabled or no longer bound to an identity"
    );
}

#[tokio::test]
async fn resolve_agent_without_binding_is_denied() {
    let fake = FakeRecords::shared();
    {
        let mut agent = autonomous_agent();
        agent.identity = None;
        let mut state = fake.state();
        state.agent = Some(agent);
        state.identity = Some(binding("agent-1"));
    }
    let failure = store(&fake)
        .resolve_verified_claims(&application_claims())
        .await
        .expect_err("an agent without a binding");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "Agent is disabled or no longer bound to an identity"
    );
}

#[tokio::test]
async fn resolve_inactive_binding_is_denied() {
    let fake = FakeRecords::shared();
    {
        let mut agent = autonomous_agent();
        agent.identity.as_mut().expect("the fixture binding").active = PolicyFlag(false);
        let mut state = fake.state();
        state.agent = Some(agent);
        state.identity = Some(binding("agent-1"));
    }
    let failure = store(&fake)
        .resolve_verified_claims(&application_claims())
        .await
        .expect_err("an inactive binding");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "Agent is disabled or no longer bound to an identity"
    );
}

#[tokio::test]
async fn resolve_application_token_returns_autonomous_context() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.agent = Some(autonomous_agent());
        state.identity = Some(binding("agent-1"));
    }
    let context = store(&fake)
        .resolve_verified_claims(&application_claims())
        .await
        .expect("the claim resolution")
        .expect("the managed context");
    assert_eq!(context.agent_id, "agent-1");
    assert_eq!(context.binding_revision.as_deref(), Some("rev-1"));
    assert_eq!(context.mode, ManagedAgentMode::Autonomous);
    assert!(context.user_id.is_none());
    assert_eq!(context.subject_oid.as_deref(), Some("oid-1"));
    assert!(fake.state().subject_reads.is_empty());
}

#[tokio::test]
async fn resolve_delegated_token_needs_a_proven_human() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.agent = Some(delegated_agent());
        state.identity = Some(binding("agent-2"));
    }
    let failure = store(&fake)
        .resolve_verified_claims(&delegated_claims())
        .await
        .expect_err("an unproven delegated user");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "The delegated user must first sign in through trusted Microsoft SSO"
    );
    assert_eq!(
        fake.state().subject_reads,
        vec![RecordedSubjectKey {
            issuer: String::from("issuer-1"),
            tenant_id: String::from("tenant-1"),
            oid: String::from("oid-2"),
        }]
    );
}

#[tokio::test]
async fn resolve_delegated_token_returns_the_human_user() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.agent = Some(delegated_agent());
        state.identity = Some(binding("agent-2"));
        state.subject = Some(human_row("human", "sso_interactive", Some("user-2")));
    }
    let context = store(&fake)
        .resolve_verified_claims(&delegated_claims())
        .await
        .expect("the claim resolution")
        .expect("the managed context");
    assert_eq!(context.agent_id, "agent-2");
    assert_eq!(context.binding_revision.as_deref(), Some("rev-2"));
    assert_eq!(context.mode, ManagedAgentMode::Delegated);
    assert_eq!(context.user_id.as_deref(), Some("user-2"));
    assert_eq!(context.subject_oid.as_deref(), Some("oid-2"));
}

#[tokio::test]
async fn resolve_delegated_subject_of_another_kind_is_denied() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.agent = Some(delegated_agent());
        state.identity = Some(binding("agent-2"));
        state.subject = Some(human_row("application", "sso_interactive", Some("user-2")));
    }
    let failure = store(&fake)
        .resolve_verified_claims(&delegated_claims())
        .await
        .expect_err("a non-human subject");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "The delegated user must first sign in through trusted Microsoft SSO"
    );
}

#[tokio::test]
async fn resolve_delegated_subject_not_interactive_is_denied() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.agent = Some(delegated_agent());
        state.identity = Some(binding("agent-2"));
        state.subject = Some(human_row("human", "sso_other", Some("user-2")));
    }
    let failure = store(&fake)
        .resolve_verified_claims(&delegated_claims())
        .await
        .expect_err("a subject verified another way");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "The delegated user must first sign in through trusted Microsoft SSO"
    );
}

#[tokio::test]
async fn resolve_delegated_subject_without_user_is_denied() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.agent = Some(delegated_agent());
        state.identity = Some(binding("agent-2"));
        state.subject = Some(human_row("human", "sso_interactive", None));
    }
    let failure = store(&fake)
        .resolve_verified_claims(&delegated_claims())
        .await
        .expect_err("a subject without a user");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "The delegated user must first sign in through trusted Microsoft SSO"
    );
}

#[tokio::test]
async fn resolve_subject_read_failure_is_policy_unavailable() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.agent = Some(delegated_agent());
        state.identity = Some(binding("agent-2"));
        state.failure = Some(FakeCall::Subject);
    }
    let failure = store(&fake)
        .resolve_verified_claims(&delegated_claims())
        .await
        .expect_err("a subject read failure");
    assert_eq!(failure.code, FailureCode::PolicyUnavailable);
    assert_eq!(failure.message, "Subject classification is unavailable");
}

#[tokio::test]
async fn resolve_managed_agent_without_a_client_is_unbound() {
    let context = super::resolve_managed_agent(&application_claims(), None, None)
        .await
        .expect("an unbound claim");
    assert!(context.is_none());
}

#[tokio::test]
async fn resolve_managed_agent_without_a_database_is_policy_unavailable() {
    let client = PostgresStore { pool: None };
    let failure = super::resolve_managed_agent(&application_claims(), Some(&client), None)
        .await
        .expect_err("a policy failure");
    assert_eq!(failure.status, POLICY_UNAVAILABLE_STATUS);
    assert_eq!(failure.message, "Agent identity could not be loaded");
}
