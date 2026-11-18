use deadpool_postgres::GenericClient;

const FIND_SQL: &str = "SELECT COALESCE(\"blocked_tools\", ARRAY[]::text[]) AS blocked_tools FROM \"LiteLLM_ObjectPermissionTable\" WHERE \"object_permission_id\" = $1";
const UPDATE_SQL: &str = "UPDATE \"LiteLLM_ObjectPermissionTable\" SET \"blocked_tools\" = $2 WHERE \"object_permission_id\" = $1";

#[derive(Clone, Copy)]
enum BlockedChange {
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

// ast-grep-ignore: no-long-names, no-one-line-functions
pub async fn add_tool_to_object_permission_blocked<Client: GenericClient + Sync>(
    client: Option<&Client>,
    object_permission_id: &str,
    tool_name: &str,
) -> bool {
    change_blocked_tools(client, object_permission_id, tool_name, BlockedChange::Add).await
}

// ast-grep-ignore: no-long-names
pub async fn remove_tool_from_object_permission_blocked<Client: GenericClient + Sync>(
    client: Option<&Client>,
    object_permission_id: &str,
    tool_name: &str,
) -> bool {
    change_blocked_tools(
        client,
        object_permission_id,
        tool_name,
        BlockedChange::Remove,
    )
    .await
}

async fn change_blocked_tools<Client: GenericClient + Sync>(
    client: Option<&Client>,
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
    let mut current = match blocked_tools_of(connected, object_permission_id).await {
        Ok(Some(current)) => current,
        Ok(None) => return false,
        Err(error) => {
            tracing::error!("tool_registry_writer {} error: {error}", change.operation());
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
    match connected
        .execute(UPDATE_SQL, &[&object_permission_id, &current])
        .await
    {
        Ok(_) => true,
        Err(error) => {
            tracing::error!("tool_registry_writer {} error: {error}", change.operation());
            false
        }
    }
}

async fn blocked_tools_of<Client: GenericClient + Sync>(
    client: &Client,
    object_permission_id: &str,
) -> Result<Option<Vec<String>>, tokio_postgres::Error> {
    let row = client.query_opt(FIND_SQL, &[&object_permission_id]).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    row.try_get("blocked_tools").map(Some)
}
