use serde::Deserialize;

use super::decode::row_as;
use super::{
    ChildTable, DeletedServer, ServerWriteError, ServerWriteStore, TokenCacheInvalidation,
};

#[derive(Deserialize)]
struct DeletedRow {
    server_id: String,
    #[serde(default)]
    server_name: Option<String>,
}

// LiteLLM's delete_mcp_server: the row delete is the commit point, every per-user cleanup after it
// is best effort, because a failure there leaves only orphaned rows pointing at a missing server.
pub async fn delete_mcp_server<S, I>(
    store: &S,
    server_id: &str,
    invalidation: &I,
) -> Result<Option<DeletedServer>, ServerWriteError>
where
    S: ServerWriteStore,
    I: TokenCacheInvalidation,
{
    let Some(row) = store.delete_server(server_id).await? else {
        return Ok(None);
    };
    let deleted: DeletedRow = row_as(&row)?;
    let user_ids = enumerated_credential_users(store, server_id).await;
    drop_children(store, server_id).await;
    // The caches are keyed by (user_id, server_id), so a server re-created under the same id must
    // not serve tokens minted for the deleted one.
    for user_id in &user_ids {
        if let Err(error) = invalidation.invalidate(user_id, server_id).await {
            tracing::warn!(
                "MCP server {server_id} deleted but the cached OAuth token of user {user_id} could not be invalidated; it expires by TTL: {error}"
            );
        }
    }
    Ok(Some(DeletedServer {
        server_id: deleted.server_id,
        server_name: deleted.server_name,
    }))
}

// The per-server child tables carry no foreign key, so each is cleaned independently of the others.
pub(crate) async fn drop_children<S>(store: &S, server_id: &str)
where
    S: ServerWriteStore,
{
    for table in [
        ChildTable::Credential,
        ChildTable::EnvVar,
        ChildTable::OAuthClient,
    ] {
        if let Err(error) = store.delete_children(table, server_id).await {
            tracing::warn!(
                "MCP server {server_id} deleted but per-user {} cleanup failed; orphaned rows can be removed on a later delete: {error}",
                table.label()
            );
        }
    }
}

async fn enumerated_credential_users<S>(store: &S, server_id: &str) -> Vec<String>
where
    S: ServerWriteStore,
{
    match store.credential_user_ids(server_id).await {
        Ok(user_ids) => user_ids,
        Err(error) => {
            tracing::warn!(
                "MCP server {server_id} deleted but per-user credential enumeration failed; cached tokens expire by TTL: {error}"
            );
            Vec::new()
        }
    }
}
