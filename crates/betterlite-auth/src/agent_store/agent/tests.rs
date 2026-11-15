#![cfg(test)]

use crate::agent_store::agent;
use crate::agent_store::tests::support::{FakeCall, FakeRecords, autonomous_agent, fake_store};
use crate::identity::FailureCode;

#[tokio::test]
async fn agent_returns_the_stored_row() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.agent = Some(autonomous_agent());
    }
    let store = fake_store(&fake, None);
    let found = agent::load(&store, "agent-1")
        .await
        .expect("the agent read");
    let stored = found.expect("the stored agent");
    assert_eq!(stored.agent_id, "agent-1");
    assert!(stored.identity.is_some());
    assert!(stored.object_permission.is_some());
    assert_eq!(fake.state().agent_reads, vec![String::from("agent-1")]);
}

#[tokio::test]
async fn agent_missing_returns_none() {
    let fake = FakeRecords::shared();
    let store = fake_store(&fake, None);
    let found = agent::load(&store, "agent-1")
        .await
        .expect("the agent read");
    assert!(found.is_none());
}

#[tokio::test]
async fn agent_read_failure_is_policy_unavailable() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.failure = Some(FakeCall::Agent);
    }
    let store = fake_store(&fake, None);
    let failure = agent::load(&store, "agent-1")
        .await
        .expect_err("a policy failure");
    assert_eq!(failure.code, FailureCode::PolicyUnavailable);
    assert_eq!(failure.message, "Agent policy could not be loaded");
}
