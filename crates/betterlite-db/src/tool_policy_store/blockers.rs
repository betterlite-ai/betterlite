use deadpool_postgres::GenericClient;

const FIND_SQL: &str = "SELECT COALESCE(\"blocked_tools\", ARRAY[]::text[]) AS blocked_tools FROM \"LiteLLM_ObjectPermissionTable\" WHERE \"object_permission_id\" = $1";
const UPDATE_SQL: &str = "UPDATE \"LiteLLM_ObjectPermissionTable\" SET \"blocked_tools\" = $2 WHERE \"object_permission_id\" = $1";

pub async fn add_tool_to_object_permission_blocked<Client>(
    client: Option<&Client>,
    object_permission_id: &str,
    tool_name: &str,
) -> bool
where
    Client: GenericClient + Sync,
{
    if object_permission_id.is_empty() || tool_name.is_empty() {
        return false;
    }
    let Some(connected) = client else {
        tracing::error!(
            "tool_registry_writer add_tool_to_object_permission_blocked error: no database connection"
        );
        return false;
    };
    let current = match blocked_tools_of(connected, object_permission_id).await {
        Ok(Some(current)) => current,
        Ok(None) => return false,
        Err(error) => {
            tracing::error!(
                "tool_registry_writer add_tool_to_object_permission_blocked error: {error}"
            );
            return false;
        }
    };
    if current.iter().any(|blocked| blocked.as_str() == tool_name) {
        return true;
    }
    let mut listed = current;
    listed.push(tool_name.to_owned());
    match connected
        .execute(UPDATE_SQL, &[&object_permission_id, &listed])
        .await
    {
        Ok(_) => true,
        Err(error) => {
            tracing::error!(
                "tool_registry_writer add_tool_to_object_permission_blocked error: {error}"
            );
            false
        }
    }
}

pub async fn remove_tool_from_object_permission_blocked<Client>(
    client: Option<&Client>,
    object_permission_id: &str,
    tool_name: &str,
) -> bool
where
    Client: GenericClient + Sync,
{
    if object_permission_id.is_empty() || tool_name.is_empty() {
        return false;
    }
    let Some(connected) = client else {
        tracing::error!(
            "tool_registry_writer remove_tool_from_object_permission_blocked error: no database connection"
        );
        return false;
    };
    let current = match blocked_tools_of(connected, object_permission_id).await {
        Ok(Some(current)) => current,
        Ok(None) => return false,
        Err(error) => {
            tracing::error!(
                "tool_registry_writer remove_tool_from_object_permission_blocked error: {error}"
            );
            return false;
        }
    };
    if !current.iter().any(|blocked| blocked.as_str() == tool_name) {
        return false;
    }
    let remaining: Vec<String> = current
        .into_iter()
        .filter(|blocked| blocked.as_str() != tool_name)
        .collect();
    match connected
        .execute(UPDATE_SQL, &[&object_permission_id, &remaining])
        .await
    {
        Ok(_) => true,
        Err(error) => {
            tracing::error!(
                "tool_registry_writer remove_tool_from_object_permission_blocked error: {error}"
            );
            false
        }
    }
}

async fn blocked_tools_of<Client>(
    client: &Client,
    object_permission_id: &str,
) -> Result<Option<Vec<String>>, tokio_postgres::Error>
where
    Client: GenericClient + Sync,
{
    let row = client.query_opt(FIND_SQL, &[&object_permission_id]).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    row.try_get("blocked_tools").map(Some)
}
