use betterlite_core::agents::AgentConfig;
use betterlite_core::agents::AgentResponse;
use uuid::Uuid;

use super::AgentChangePublisher;
use super::AgentStore;
use super::error::AgentPersistenceError;
use super::error::open_failure;
use super::error::write_error;
use super::identity_write::history_should_insert;
use super::permission_write::prepare;
use super::row::RowWrite;
use super::row::read_agent;
use super::row::write_rows;
use super::write_plan::AgentWritePlan;

pub(super) async fn store_agent<P: AgentChangePublisher>(
    store: &AgentStore<P>,
    agent: &AgentConfig,
    plan: &AgentWritePlan,
    agent_id: Option<&str>,
) -> Result<AgentResponse, AgentPersistenceError> {
    let permission = prepare(&store.store, agent.object_permission.as_ref(), None).await?;
    let insert = history_should_insert(&store.store, plan.history.as_ref(), None).await?;
    let target_id = agent_id.map_or_else(|| Uuid::new_v4().to_string(), String::from);
    let mut client = store.store.session().await.map_err(open_failure)?;
    let tx = client.transaction().await.map_err(write_error)?;
    let write = RowWrite {
        plan,
        agent_id: &target_id,
        permission: permission.as_ref(),
        link_permission: permission
            .as_ref()
            .map(|prepared| prepared.object_permission_id.as_str()),
        history: insert,
        insert: true,
    };
    write_rows(&tx, &write).await?;
    let record = read_agent(&tx, &target_id).await?.ok_or_else(|| {
        AgentPersistenceError::NotFound(format!("Agent not found, passed agent_id={target_id}"))
    })?;
    tx.commit().await.map_err(write_error)?;
    store.publisher.agents_changed().await;
    Ok(record.agent)
}

mod tests;
