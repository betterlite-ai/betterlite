use async_trait::async_trait;

// B725's object_permission row looked up by id; a lookup failure is swallowed by the caller below.
#[derive(Debug, thiserror::Error)]
pub enum PermissionLookupError {
    #[error("the object permission row could not be read")]
    Unreadable,
}

#[async_trait]
pub trait ObjectPermissions: Send + Sync {
    async fn mcp_servers(
        &self,
        object_permission_id: &str,
    ) -> Result<Option<Vec<String>>, PermissionLookupError>;
}

// get_mcp_server_ids: no DB, no permission id, no row, no servers or any query error all yield no ids.
#[must_use]
pub async fn get_mcp_server_ids(
    permissions: Option<&dyn ObjectPermissions>,
    object_permission_id: Option<&str>,
) -> Vec<String> {
    let Some(permissions) = permissions else {
        return Vec::new();
    };
    let Some(object_permission_id) = object_permission_id else {
        return Vec::new();
    };
    match permissions.mcp_servers(object_permission_id).await {
        Ok(Some(servers)) => servers,
        Ok(None) | Err(_failure) => Vec::new(),
    }
}
