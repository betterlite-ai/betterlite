#![cfg(test)]

use crate::agent_store::authentication;
use crate::agent_store::records::AgentStateFilter;
use crate::agent_store::tests::support::{
    FakeCall, FakeRecords, RecordedAuthentication, authentication_context, fake_store, fixed_clock,
};
use crate::identity::FailureCode;

#[tokio::test]
async fn record_authentication_none_revision_is_a_failure() {
    let fake = FakeRecords::shared();
    let store = fake_store(&fake, None);
    let context = authentication_context("agent-1", None);
    let failure = authentication::record_authentication(&store, &context)
        .await
        .expect_err("a missing binding revision");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "Agent authentication requires a binding revision"
    );
    assert!(fake.state().authentications.is_empty());
}

#[tokio::test]
async fn record_authentication_checks_the_agent_filter() {
    let fake = FakeRecords::shared();
    fake.state().authentication_count = 1;
    let store = fake_store(&fake, None);
    let context = authentication_context("agent-1", Some("rev-1"));
    authentication::record_authentication(&store, &context)
        .await
        .expect("the recorded authentication");
    assert_eq!(
        fake.state().authentications,
        vec![RecordedAuthentication {
            agent_id: String::from("agent-1"),
            revision: String::from("rev-1"),
            active: true,
            agent: AgentStateFilter {
                enabled: true,
                identity_managed: true,
            },
            at: fixed_clock(),
        }]
    );
}

#[tokio::test]
async fn record_authentication_disabled_agent_counts_zero() {
    let fake = FakeRecords::shared();
    fake.state().authentication_count = 0;
    let store = fake_store(&fake, None);
    let context = authentication_context("agent-1", Some("rev-1"));
    let failure = authentication::record_authentication(&store, &context)
        .await
        .expect_err("a disabled agent");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "Agent identity changed during authentication; retry"
    );
    assert_eq!(fake.state().authentications.len(), 1);
}

#[tokio::test]
async fn record_authentication_two_matches_is_a_failure() {
    let fake = FakeRecords::shared();
    fake.state().authentication_count = 2;
    let store = fake_store(&fake, None);
    let context = authentication_context("agent-1", Some("rev-1"));
    let failure = authentication::record_authentication(&store, &context)
        .await
        .expect_err("a duplicated binding");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "Agent identity changed during authentication; retry"
    );
}

#[tokio::test]
async fn record_authentication_read_failure_is_policy_unavailable() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.failure = Some(FakeCall::Authentication);
    }
    let store = fake_store(&fake, None);
    let context = authentication_context("agent-1", Some("rev-1"));
    let failure = authentication::record_authentication(&store, &context)
        .await
        .expect_err("an update failure");
    assert_eq!(failure.code, FailureCode::PolicyUnavailable);
    assert_eq!(
        failure.message,
        "Agent authentication could not be recorded"
    );
}
