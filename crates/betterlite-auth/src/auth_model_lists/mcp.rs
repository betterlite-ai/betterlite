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
#[must_use]
pub async fn get_mcp_server_ids(
    permissions: Option<&dyn ObjectPermissions>,
    object_permission_id: Option<&str>,
) -> Vec<String> {
    let Some(store) = permissions else {
        return Vec::new();
    };
    let Some(permission_id) = object_permission_id else {
        return Vec::new();
    };
    let found = match store.mcp_servers(permission_id).await {
        Ok(servers) => servers,
        Err(_failure) => None,
    };
    found.unwrap_or_default()
}
