use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use tracing::warn;

use super::Ceilings;
use super::error::{CeilingReadError, CeilingsError, CeilingsResult};
use super::reads::CeilingReads;
use super::servers::{self, ToolsetTerm};
use super::tools::tool_grants_for_server;
use crate::auth::object_permission_hydration::reads::ObjectPermission;
use crate::auth::object_permission_hydration::record::McpAuth;

// the agent's own row: no agent, no key or an unreadable key reads as nothing, and a prefetched row wins over a fresh read
async fn agent_permission<Reads>(
    ceilings: &Ceilings<Reads>,
    auth: Option<&UserAPIKeyAuth>,
    prefetched: Option<&ObjectPermission>,
) -> Result<Option<ObjectPermission>, CeilingReadError<Reads::Error>>
where
    Reads: CeilingReads,
{
    let Some(caller) = auth else {
        return Ok(None);
    };
    let Some(record) = McpAuth::from_auth(caller).ok() else {
        return Ok(None);
    };
    if record.agent_id.is_none() {
        return Ok(None);
    }
    if let Some(permission) = prefetched {
        return Ok(Some(permission.clone()));
    }
    ceilings.reads.agent_object_permission(Some(caller)).await
}

// get_allowed_mcp_servers_for_agent: a plain agent reads a union fault as no restriction, a managed one or an unloadable row returns it
pub(super) async fn agent_servers<Reads>(
    ceilings: &Ceilings<Reads>,
    auth: Option<&UserAPIKeyAuth>,
    prefetched: Option<&ObjectPermission>,
) -> CeilingsResult<Vec<String>, Reads>
where
    Reads: CeilingReads,
{
    let Some(permission) = agent_permission(ceilings, auth, prefetched).await? else {
        return Ok(Vec::new());
    };
    let fresh = ceilings.reads.requires_fresh_policy(auth);
    let listed =
        servers::permission_servers(&ceilings.reads, &permission, fresh, ToolsetTerm::Counted);
    match listed.await {
        Ok(server_ids) => Ok(server_ids),
        Err(error) => agent_fault(ceilings, auth, error, Vec::new()),
    }
}

// _get_agent_access_group_server_ceiling: an attached group set that names no server still denies every server
pub(super) async fn agent_access_group_ceiling<Reads>(
    ceilings: &Ceilings<Reads>,
    auth: Option<&UserAPIKeyAuth>,
) -> CeilingsResult<Option<Vec<String>>, Reads>
where
    Reads: CeilingReads,
{
    let Some(record) = auth.and_then(|caller| McpAuth::from_auth(caller).ok()) else {
        return Ok(None);
    };
    let Some(agent_id) = record.agent_id else {
        return Ok(None);
    };
    let Some(server_ids) = ceilings
        .reads
        .agent_access_group_server_ids(&agent_id)
        .await?
    else {
        return Ok(None);
    };
    let mut sorted = server_ids;
    sorted.sort();
    let expanded = ceilings.reads.expand_permission_list(&sorted);
    Ok(Some(servers::distinct(expanded)))
}

// _agent_capped_servers: an empty agent list is no restriction, while an empty access group ceiling denies every server
#[must_use]
pub fn agent_capped_servers(
    allowed: &[String],
    agent_servers: &[String],
    access_group_servers: Option<&[String]>,
) -> Option<Vec<String>> {
    if agent_servers.is_empty() && access_group_servers.is_none() {
        return None;
    }
    let kept = allowed.iter().filter(|server| {
        (agent_servers.is_empty() || agent_servers.contains(*server))
            && access_group_servers.is_none_or(|ceiling| ceiling.contains(*server))
    });
    Some(kept.cloned().collect())
}

// the server whose agent tool grants are read, and the caller and row the read starts from
pub(super) struct AgentServer<'a> {
    pub(super) server_id: &'a str,
    pub(super) auth: Option<&'a UserAPIKeyAuth>,
    pub(super) prefetched: Option<&'a ObjectPermission>,
}

// get_agent_tool_permissions_for_server: direct grants count only when the row lists any, and a plain agent reads a fault as no restriction
pub(super) async fn agent_server_tools<Reads>(
    ceilings: &Ceilings<Reads>,
    server: AgentServer<'_>,
) -> CeilingsResult<Option<Vec<String>>, Reads>
where
    Reads: CeilingReads,
{
    let Some(permission) = agent_permission(ceilings, server.auth, server.prefetched).await? else {
        return Ok(None);
    };
    let direct = permission
        .mcp_tool_permissions
        .as_ref()
        .filter(|grants| !grants.is_empty());
    let fresh = ceilings.reads.requires_fresh_policy(server.auth);
    let granted = tool_grants_for_server(
        &ceilings.reads,
        Some(&permission),
        direct,
        server.server_id,
        fresh,
    )
    .await;
    granted.or_else(|error| agent_fault(ceilings, server.auth, error, None))
}

// a plain agent reads a fault as no restriction, while a managed agent or an unloadable row returns the error
pub(super) fn agent_fault<Reads, Value>(
    ceilings: &Ceilings<Reads>,
    auth: Option<&UserAPIKeyAuth>,
    error: CeilingReadError<Reads::Error>,
    recovered: Value,
) -> CeilingsResult<Value, Reads>
where
    Reads: CeilingReads,
{
    if matches!(error, CeilingReadError::Unloadable(_)) || ceilings.reads.is_managed_agent(auth) {
        return Err(CeilingsError::from(error));
    }
    warn!("Failed to read the agent MCP entitlement: {error}");
    Ok(recovered)
}
