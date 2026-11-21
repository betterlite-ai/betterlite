#![cfg(test)]

use super::AgentPersistenceError;
use crate::agent_persistence::support::Database;
use crate::agent_persistence::support::IDENTITY;
use crate::agent_persistence::support::RecordingPublisher;
use crate::agent_persistence::support::agent;
use crate::agent_persistence::support::count_where;
use crate::agent_persistence::support::store_with;

const AGENTS: &str = "LiteLLM_AgentsTable";
const RETIRED: &str = "LiteLLM_RetiredAgent";
const TOKENS: &str = "LiteLLM_VerificationToken";

#[tokio::test]
async fn remove_retires_a_managed_agent_and_deletes_its_keys() {
    let database = Database::start().await;
    let publisher = RecordingPublisher::default();
    let store = store_with(&database, publisher.clone());
    let config = agent(&format!(r#"{{"agent_name": "a", {IDENTITY}}}"#));
    let created = store
        .add_agent_to_db(&config, "author", None)
        .await
        .expect("the agent is created");
    database
        .execute(
            r#"INSERT INTO "LiteLLM_VerificationToken" (token, agent_id) VALUES ($1, $2)"#,
            &[&"key-1", &created.agent_id],
        )
        .await;
    store
        .delete_agent_from_db(&created.agent_id)
        .await
        .expect("the agent is deleted");
    let agents = count_where(&database, AGENTS, "agent_id", &created.agent_id).await;
    assert_eq!(agents, 0);
    let retired = count_where(&database, RETIRED, "original_agent_id", &created.agent_id).await;
    assert_eq!(retired, 1);
    let keys = count_where(&database, TOKENS, "agent_id", &created.agent_id).await;
    assert_eq!(keys, 0);
    assert_eq!(publisher.calls(), 1);
    database.close().await;
}

#[tokio::test]
async fn remove_keeps_the_keys_of_an_unmanaged_agent() {
    let database = Database::start().await;
    let publisher = RecordingPublisher::default();
    let store = store_with(&database, publisher.clone());
    let config = agent(r#"{"agent_name": "a"}"#);
    let created = store
        .add_agent_to_db(&config, "author", None)
        .await
        .expect("the agent is created");
    database
        .execute(
            r#"INSERT INTO "LiteLLM_VerificationToken" (token, agent_id) VALUES ($1, $2)"#,
            &[&"key-1", &created.agent_id],
        )
        .await;
    store
        .delete_agent_from_db(&created.agent_id)
        .await
        .expect("the agent is deleted");
    let retired = count_where(&database, RETIRED, "original_agent_id", &created.agent_id).await;
    assert_eq!(retired, 0);
    let keys = count_where(&database, TOKENS, "agent_id", &created.agent_id).await;
    assert_eq!(keys, 1);
    assert_eq!(publisher.calls(), 1);
    database.close().await;
}

#[tokio::test]
async fn remove_reports_a_missing_agent() {
    let database = Database::start().await;
    let publisher = RecordingPublisher::default();
    let store = store_with(&database, publisher.clone());
    let failure = store
        .delete_agent_from_db("missing")
        .await
        .expect_err("the agent does not exist");
    let AgentPersistenceError::NotFound(text) = failure else {
        panic!("unexpected failure: {failure}");
    };
    assert_eq!(text, "Agent not found, passed agent_id=missing");
    let retired = count_where(&database, RETIRED, "original_agent_id", "missing").await;
    assert_eq!(retired, 0);
    assert_eq!(publisher.calls(), 0);
    database.close().await;
}
