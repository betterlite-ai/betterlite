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
    let (Some(store), Some(permission_id)) = (permissions, object_permission_id) else {
        return Vec::new();
    };
    match store.mcp_servers(permission_id).await {
        Ok(servers) => servers.unwrap_or_default(),
        Err(_failure) => Vec::new(),
    }
}
