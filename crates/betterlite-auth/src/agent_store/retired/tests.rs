#![cfg(test)]

use crate::agent_store::records::RetiredAgentRecord;
use crate::agent_store::tests::support::{
    FakeCall, FakeRecords, store, store_without_retired_agents,
};
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
    let retired = store(&fake)
        .retired_agent("agent-1")
        .await
        .expect("the history check");
    assert!(retired);
    assert_eq!(
        fake.state().retired_agent_reads,
        vec![String::from("agent-1")]
    );
}

#[tokio::test]
async fn retired_agent_false_without_a_row() {
    let fake = FakeRecords::shared();
    let retired = store(&fake)
        .retired_agent("agent-1")
        .await
        .expect("the history check");
    assert!(!retired);
}

#[tokio::test]
async fn retired_agent_without_a_repository_is_policy_unavailable() {
    let fake = FakeRecords::shared();
    let failure = store_without_retired_agents(&fake)
        .retired_agent("agent-1")
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
    let failure = store(&fake)
        .retired_agent("agent-1")
        .await
        .expect_err("a history read failure");
    assert_eq!(failure.code, FailureCode::PolicyUnavailable);
    assert_eq!(failure.message, "Agent history is unavailable");
}
