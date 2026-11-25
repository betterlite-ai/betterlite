use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use tracing::warn;

use super::Ceilings;
use super::error::CeilingsResult;
use super::reads::CeilingReads;
use super::servers::{self, ToolsetTerm};
use crate::auth::object_permission_hydration::record::McpAuth;

// _get_allowed_mcp_servers_for_end_user: this level adds no toolset servers, and a union fault reads as no restriction
pub(super) async fn end_user_servers<Reads>(
    ceilings: &Ceilings<Reads>,
    auth: Option<&UserAPIKeyAuth>,
) -> CeilingsResult<Vec<String>, Reads>
where
    Reads: CeilingReads,
{
    let Some(caller) = auth else {
        return Ok(Vec::new());
    };
    let Some(record) = McpAuth::from_auth(caller).ok() else {
        return Ok(Vec::new());
    };
    if record.end_user_id.is_none() || !ceilings.reads.has_database() {
        return Ok(Vec::new());
    }
    let loaded = ceilings.reads.end_user_object_permission(auth).await?;
    let Some(permission) = loaded else {
        return Ok(Vec::new());
    };
    let fresh = ceilings.reads.requires_fresh_policy(auth);
    match servers::permission_servers(&ceilings.reads, &permission, fresh, ToolsetTerm::Skipped)
        .await
    {
        Ok(server_ids) => Ok(server_ids),
        Err(error) => {
            warn!("Failed to get allowed MCP servers for end user: {error}");
            Ok(Vec::new())
        }
    }
}
