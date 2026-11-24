use betterlite_db::auxiliary_repository::repository::PrismaClient;
use thiserror::Error;

use crate::mcp_permission_ids::store::ExistingServersError;
use crate::mcp_permission_ids::store::existing_object_permission_mcp_servers as stored_server_ids; // ast-grep-ignore: no-long-names - the LiteLLM helper this task reuses
use crate::mcp_permission_ids::team::ALL_PROXY_MCP_SERVERS;

use super::denial::Denial;

#[derive(Debug, Error)]
pub enum AdminGrantError {
    #[error(transparent)]
    Existing(#[from] ExistingServersError),
    #[error(transparent)]
    Denied(#[from] Denial),
}

pub async fn guard_all_proxy_team_grant(
    requested_mcp_servers: Option<&[String]>,
    existing_object_permission_id: Option<&str>,
    is_proxy_admin: bool,
    client: Option<PrismaClient<'_>>,
) -> Result<(), AdminGrantError> {
    let requests_sentinel = requested_mcp_servers
        .is_some_and(|servers| servers.iter().any(|server| server == ALL_PROXY_MCP_SERVERS));
    if is_proxy_admin || !requests_sentinel {
        return Ok(());
    }
    let existing = stored_server_ids(existing_object_permission_id, client).await?;
    if existing
        .iter()
        .any(|server| server == ALL_PROXY_MCP_SERVERS)
    {
        return Ok(());
    }
    Err(AdminGrantError::Denied(Denial {
        detail: String::from(
            "Only a proxy admin can grant a team access to all proxy MCP servers ('all-proxy-mcpservers').",
        ),
    }))
}
