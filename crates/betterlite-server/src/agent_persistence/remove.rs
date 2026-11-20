use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::DbSession;
use deadpool_postgres::Client;

use super::error::AgentPersistenceError;
use super::row::read_agent;

const RETIRE_SQL: &str = r#"INSERT INTO "LiteLLM_RetiredAgent" (original_agent_id) VALUES ($1) ON CONFLICT (original_agent_id) DO NOTHING"#;
const DELETE_KEYS_SQL: &str = r#"DELETE FROM "LiteLLM_VerificationToken" WHERE agent_id = $1"#;
const DELETE_AGENT_SQL: &str = r#"DELETE FROM "LiteLLM_AgentsTable" WHERE agent_id = $1"#;

async fn delete_statement(
    tx: &DbSession<'_>,
    agent_id: &str,
    sql: &str,
) -> Result<u64, AgentPersistenceError> {
    tx.execute(sql, &[&agent_id])
        .await
        .map_err(|source| AgentPersistenceError::Database(backend(source)))
}

pub(super) async fn delete_agent(
    client: &mut Client,
    agent_id: &str,
    missing: &str,
) -> Result<(), AgentPersistenceError> {
    let tx = client
        .transaction()
        .await
        .map_err(|source| AgentPersistenceError::Database(backend(source)))?;
    let Some(record) = read_agent(&tx, agent_id).await? else {
        return Err(AgentPersistenceError::NotFound(String::from(missing)));
    };
    if record.agent.identity_managed {
        delete_statement(&tx, agent_id, RETIRE_SQL).await?;
        delete_statement(&tx, agent_id, DELETE_KEYS_SQL).await?;
    }
    let deleted = delete_statement(&tx, agent_id, DELETE_AGENT_SQL).await?;
    if deleted == 0 {
        return Err(AgentPersistenceError::NotFound(String::from(missing)));
    }
    tx.commit()
        .await
        .map_err(|source| AgentPersistenceError::Database(backend(source)))?;
    Ok(())
}

mod tests;
