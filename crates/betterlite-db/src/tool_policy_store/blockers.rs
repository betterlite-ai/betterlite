use deadpool_postgres::Client;

use super::deadline::{StatementBounds, StatementFailure, bounded_statement, discard_uncancelled};

const FIND_SQL: &str = "SELECT COALESCE(\"blocked_tools\", ARRAY[]::text[]) AS blocked_tools FROM \"LiteLLM_ObjectPermissionTable\" WHERE \"object_permission_id\" = $1";
const UPDATE_SQL: &str = "UPDATE \"LiteLLM_ObjectPermissionTable\" SET \"blocked_tools\" = $2 WHERE \"object_permission_id\" = $1";

#[derive(Clone, Copy, Debug)]
pub enum BlockedChange {
    Add,
    Remove,
}

impl BlockedChange {
    const fn operation(self) -> &'static str {
        match self {
            Self::Add => "add_tool_to_object_permission_blocked",
            Self::Remove => "remove_tool_from_object_permission_blocked",
        }
    }
}

pub async fn change_blocked_tools(
    client: Option<Client>,
    bounds: StatementBounds<'_>,
    object_permission_id: &str,
    tool_name: &str,
    change: BlockedChange,
) -> bool {
    let operation = change.operation();
    if object_permission_id.is_empty() || tool_name.is_empty() {
        return false;
    }
    let Some(connected) = client else {
        tracing::error!("tool_registry_writer {operation} error: no database connection");
        return false;
    };
    let mut current = match read_blocked_list(&connected, bounds, object_permission_id).await {
        Ok(Some(current)) => current,
        Ok(None) => return false,
        Err(failure) => {
            tracing::error!("tool_registry_writer {operation} error: {failure}");
            discard_uncancelled(connected, &failure);
            return false;
        }
    };
    let listed = current.iter().any(|blocked| blocked.as_str() == tool_name);
    match (change, listed) {
        (BlockedChange::Add, true) => return true,
        (BlockedChange::Remove, false) => return false,
        (BlockedChange::Add, false) => current.push(tool_name.to_owned()),
        (BlockedChange::Remove, true) => {
            current.retain(|blocked| blocked.as_str() != tool_name);
        }
    }
    write_blocked_list(connected, bounds, object_permission_id, &current, operation).await
}

async fn write_blocked_list(
    connected: Client,
    bounds: StatementBounds<'_>,
    object_permission_id: &str,
    current: &[String],
    operation: &str,
) -> bool {
    match bounded_statement(
        &connected,
        bounds,
        connected.execute(UPDATE_SQL, &[&object_permission_id, &current]),
    )
    .await
    {
        Ok(_) => true,
        Err(failure) => {
            tracing::error!("tool_registry_writer {operation} error: {failure}");
            discard_uncancelled(connected, &failure);
            false
        }
    }
}

async fn read_blocked_list(
    client: &Client,
    bounds: StatementBounds<'_>,
    object_permission_id: &str,
) -> Result<Option<Vec<String>>, StatementFailure> {
    let found = bounded_statement(
        client,
        bounds,
        client.query_opt(FIND_SQL, &[&object_permission_id]),
    )
    .await?;
    let Some(row) = found else {
        return Ok(None);
    };
    row.try_get::<_, Vec<String>>("blocked_tools")
        .map(Some)
        .map_err(StatementFailure::Driver)
}
