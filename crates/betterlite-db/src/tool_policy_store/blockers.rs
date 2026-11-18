use std::time::Duration;

use deadpool_postgres::Client;

use super::deadline::{StatementFailure, bounded_statement};

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

enum BlockedList {
    Listed(Vec<String>),
    Missing,
    Unreadable,
}

pub async fn change_blocked_tools(
    client: Option<&Client>,
    deadline: Duration,
    object_permission_id: &str,
    tool_name: &str,
    change: BlockedChange,
) -> bool {
    if object_permission_id.is_empty() || tool_name.is_empty() {
        return false;
    }
    let Some(connected) = client else {
        tracing::error!(
            "tool_registry_writer {} error: no database connection",
            change.operation()
        );
        return false;
    };
    let operation = change.operation();
    let mut current =
        match read_blocked_list(connected, deadline, object_permission_id, operation).await {
            BlockedList::Listed(current) => current,
            BlockedList::Missing | BlockedList::Unreadable => return false,
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
    match write_blocked_list(connected, deadline, object_permission_id, &current).await {
        Ok(_) => true,
        Err(failure) => {
            tracing::error!("tool_registry_writer {operation} error: {failure}");
            false
        }
    }
}

async fn read_blocked_list(
    client: &Client,
    deadline: Duration,
    object_permission_id: &str,
    operation: &str,
) -> BlockedList {
    let found = match bounded_statement(
        client,
        deadline,
        client.query_opt(FIND_SQL, &[&object_permission_id]),
    )
    .await
    {
        Ok(found) => found,
        Err(failure) => {
            tracing::error!("tool_registry_writer {operation} error: {failure}");
            return BlockedList::Unreadable;
        }
    };
    match found {
        Some(row) => match row.try_get("blocked_tools") {
            Ok(current) => BlockedList::Listed(current),
            Err(error) => {
                tracing::error!("tool_registry_writer {operation} error: {error}");
                BlockedList::Unreadable
            }
        },
        None => BlockedList::Missing,
    }
}

async fn write_blocked_list(
    client: &Client,
    deadline: Duration,
    object_permission_id: &str,
    blocked: &[String],
) -> Result<u64, StatementFailure> {
    bounded_statement(
        client,
        deadline,
        client.execute(UPDATE_SQL, &[&object_permission_id, &blocked]),
    )
    .await
}
