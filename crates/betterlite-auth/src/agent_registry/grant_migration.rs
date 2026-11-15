use betterlite_db::auxiliary_repository::bindings::ObjectPermissionTable;
use betterlite_db::auxiliary_repository::repository::TableBinding as _;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::{DbSession, PostgresStore};
use futures_util::future::join_all;

use super::AgentRegistry;
use super::AgentRegistryApi as _;
use super::AgentRegistryError;
use super::GrantMigrationResult;

mod tests;

struct RowUpdate {
    object_permission_id: String,
    snapshot: Vec<String>,
    translated: Vec<String>,
}

pub(super) async fn migrate_legacy_grant_ids(
    registry: &AgentRegistry,
    store: &PostgresStore,
) -> Result<GrantMigrationResult, AgentRegistryError> {
    let legacy_ids = legacy_ids_to_migrate(registry);
    if legacy_ids.is_empty() {
        return Ok(GrantMigrationResult::default());
    }
    let mut client = store.session().await?;
    let session = client
        .transaction()
        .await
        .map_err(|source| AgentRegistryError::Database(backend(source)))?;
    let rows = read_rows(&session, &legacy_ids).await?;
    let updates: Vec<RowUpdate> = rows
        .into_iter()
        .map(|(object_permission_id, snapshot)| RowUpdate {
            translated: translated_agents(registry, &snapshot),
            object_permission_id,
            snapshot,
        })
        .collect();
    let results = join_all(updates.iter().map(|update| write_row(&session, update))).await;
    let mut rewritten = 0_usize;
    for count in results {
        rewritten = rewritten.saturating_add(count?);
    }
    session
        .commit()
        .await
        .map_err(|source| AgentRegistryError::Database(backend(source)))?;
    let missed = updates.len().saturating_sub(rewritten);
    Ok(GrantMigrationResult { rewritten, missed })
}

fn legacy_ids_to_migrate(registry: &AgentRegistry) -> Vec<String> {
    let state = registry.state();
    let mut ids = Vec::new();
    for (legacy, stable) in &state.legacy_to_stable {
        if legacy != stable {
            ids.push(legacy.clone());
        }
    }
    drop(state);
    ids
}

fn translated_agents(registry: &AgentRegistry, agents: &[String]) -> Vec<String> {
    let mut translated: Vec<String> = Vec::with_capacity(agents.len());
    for agent_id in agents {
        let stable = registry.stable_agent_id(agent_id);
        if !translated.contains(&stable) {
            translated.push(stable);
        }
    }
    translated
}

async fn read_rows(
    session: &DbSession<'_>,
    legacy_ids: &[String],
) -> Result<Vec<(String, Vec<String>)>, AgentRegistryError> {
    let sql = format!(
        "SELECT object_permission_id, agents FROM \"{}\" WHERE agents && $1",
        ObjectPermissionTable::TABLE
    );
    let rows = session
        .query(&sql, &[&legacy_ids])
        .await
        .map_err(|source| AgentRegistryError::Database(backend(source)))?;
    let mut found = Vec::with_capacity(rows.len());
    for row in &rows {
        let object_permission_id: String = row
            .try_get(0)
            .map_err(|source| AgentRegistryError::Database(backend(source)))?;
        let agents: Vec<String> = row
            .try_get(1)
            .map_err(|source| AgentRegistryError::Database(backend(source)))?;
        found.push((object_permission_id, agents));
    }
    Ok(found)
}

async fn write_row(
    session: &DbSession<'_>,
    update: &RowUpdate,
) -> Result<usize, AgentRegistryError> {
    let sql = format!(
        "UPDATE \"{}\" SET agents = $2 WHERE object_permission_id = $1 AND agents = $3",
        ObjectPermissionTable::TABLE
    );
    let count = session
        .execute(
            &sql,
            &[
                &update.object_permission_id,
                &update.translated,
                &update.snapshot,
            ],
        )
        .await
        .map_err(|source| AgentRegistryError::Database(backend(source)))?;
    usize::try_from(count).map_err(|source| AgentRegistryError::RowCount { source })
}
