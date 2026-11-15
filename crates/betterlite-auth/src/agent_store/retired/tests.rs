#![cfg(test)]

use crate::agent_store::records::RetiredAgentRecord;
use crate::agent_store::retired;
use crate::agent_store::tests::support::{FakeCall, FakeRecords, fake_store};
use crate::identity::FailureCode;

#[tokio::test]
async fn retired_agent_true_for_a_retired_row() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.retired_agent = Some(RetiredAgentRecord {
            original_agent_id: String::from("agent-1"),
        });
    }
    let store = fake_store(&fake, None);
    let history = retired::agent_is_retired(&store, "agent-1")
        .await
        .expect("the history check");
    assert!(history);
    assert_eq!(
        fake.state().retired_agent_reads,
        vec![String::from("agent-1")]
    );
}

#[tokio::test]
async fn retired_agent_false_without_a_row() {
    let fake = FakeRecords::shared();
    let store = fake_store(&fake, None);
    let history = retired::agent_is_retired(&store, "agent-1")
        .await
        .expect("the history check");
    assert!(!history);
}

#[tokio::test]
async fn retired_agent_without_a_repository_is_policy_unavailable() {
    let fake = FakeRecords::shared();
    let mut store = fake_store(&fake, None);
    store.retired_agents = None;
    let failure = retired::agent_is_retired(&store, "agent-1")
        .await
        .expect_err("a missing history repository");
    assert_eq!(failure.code, FailureCode::PolicyUnavailable);
    assert_eq!(failure.message, "Agent history is unavailable");
    assert!(fake.state().retired_agent_reads.is_empty());
}

#[tokio::test]
async fn retired_agent_read_failure_is_policy_unavailable() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.failure = Some(FakeCall::RetiredAgent);
    }
    let store = fake_store(&fake, None);
    let failure = retired::agent_is_retired(&store, "agent-1")
        .await
        .expect_err("a history read failure");
    assert_eq!(failure.code, FailureCode::PolicyUnavailable);
    assert_eq!(failure.message, "Agent history is unavailable");
}
