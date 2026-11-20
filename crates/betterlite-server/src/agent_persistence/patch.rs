use betterlite_core::agents::AgentResponse;
use betterlite_core::agents::permissions::AgentObjectPermission;
use betterlite_db::postgres_store::session::PostgresStore;

use crate::agent_rows::codec::StoredAgent;
use crate::object_permissions::prepare::PreparedUpsert;

use super::AgentChangePublisher;
use super::AgentStore;
use super::error::AgentPersistenceError;
use super::error::open_failure;
use super::error::write_error;
use super::identity_write::history_should_insert;
use super::permission_write::prepare;
use super::row::RowWrite;
use super::row::read_agent;
use super::row::read_agent_on;
use super::row::write_rows;
use super::write_plan::AgentWritePlan;

pub(super) struct UpdateRequest<'plan> {
    pub agent_id: &'plan str,
    pub plan: &'plan AgentWritePlan,
    pub permission: Option<&'plan AgentObjectPermission>,
    pub existing_permission: Option<&'plan str>,
    pub missing: String,
}

pub(super) async fn existing_agent(
    store: &PostgresStore,
    agent_id: &str,
) -> Result<Option<StoredAgent>, AgentPersistenceError> {
    let client = store.session().await.map_err(open_failure)?;
    let existing = read_agent_on(&client, agent_id).await?;
    drop(client);
    Ok(existing)
}

async fn prepare_permission(
    store: &PostgresStore,
    update: &UpdateRequest<'_>,
) -> Result<(Option<PreparedUpsert>, Option<String>), AgentPersistenceError> {
    let prepared = prepare(store, update.permission, update.existing_permission).await?;
    if update.existing_permission.is_some() {
        return Ok((prepared, None));
    }
    let link = prepared
        .as_ref()
        .map(|row| row.object_permission_id.clone());
    Ok((prepared, link))
}

pub(super) async fn commit_update<P: AgentChangePublisher>(
    store: &AgentStore<P>,
    update: &UpdateRequest<'_>,
) -> Result<AgentResponse, AgentPersistenceError> {
    let (permission, link) = prepare_permission(&store.store, update).await?;
    let history = history_should_insert(
        &store.store,
        update.plan.history.as_ref(),
        Some(update.agent_id),
    )
    .await?;
    let write = RowWrite {
        plan: update.plan,
        agent_id: update.agent_id,
        permission: permission.as_ref(),
        link_permission: link.as_deref(),
        history,
        insert: false,
    };
    let mut session = store.store.session().await.map_err(open_failure)?;
    let tx = session.transaction().await.map_err(write_error)?;
    let affected = write_rows(&tx, &write).await?;
    if affected == 0 {
        return Err(AgentPersistenceError::NotFound(update.missing.clone()));
    }
    let Some(record) = read_agent(&tx, update.agent_id).await? else {
        return Err(AgentPersistenceError::NotFound(update.missing.clone()));
    };
    tx.commit().await.map_err(write_error)?;
    store.publisher.agents_changed().await;
    Ok(record.agent)
}

mod tests;
