use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use tracing::warn;

use super::Ceilings;
use super::error::{CeilingReadError, CeilingsError, CeilingsResult};
use super::reads::CeilingReads;
use super::servers::{self, ToolsetTerm};
use super::tools::tool_grants_for_server;
use crate::auth::object_permission_hydration::reads::ObjectPermission;
use crate::auth::object_permission_hydration::record::McpAuth;

// get_allowed_mcp_servers_for_agent: a plain agent reads a union fault as no restriction, a managed one or an unloadable row returns it
pub(super) async fn agent_servers<Reads>(
    ceilings: &Ceilings<Reads>,
    auth: Option<&UserAPIKeyAuth>,
    prefetched: Option<&ObjectPermission>,
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
    if record.agent_id.is_none() {
        return Ok(Vec::new());
    }
    let mut loaded = prefetched.cloned();
    if loaded.is_none() {
        loaded = ceilings.reads.agent_object_permission(Some(caller)).await?;
    }
    let Some(permission) = loaded else {
        return Ok(Vec::new());
    };
    let fresh = ceilings.reads.requires_fresh_policy(Some(caller));
    match servers::permission_servers(&ceilings.reads, &permission, fresh, ToolsetTerm::Counted)
        .await
    {
        Ok(server_ids) => Ok(server_ids),
        Err(error) => agent_fault(ceilings, Some(caller), error, Vec::new()),
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

// get_agent_tool_permissions_for_server: direct grants count only when the row lists any, and a plain agent reads a fault as no restriction
pub(super) async fn agent_server_tools<Reads>(
    ceilings: &Ceilings<Reads>,
    server_id: &str,
    auth: Option<&UserAPIKeyAuth>,
    prefetched: Option<&ObjectPermission>,
) -> CeilingsResult<Option<Vec<String>>, Reads>
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
    let mut loaded = prefetched.cloned();
    if loaded.is_none() {
        loaded = ceilings.reads.agent_object_permission(Some(caller)).await?;
    }
    let Some(permission) = loaded else {
        return Ok(None);
    };
    let direct = permission
        .mcp_tool_permissions
        .as_ref()
        .filter(|grants| !grants.is_empty());
    let fresh = ceilings.reads.requires_fresh_policy(Some(caller));
    match tool_grants_for_server(&ceilings.reads, Some(&permission), direct, server_id, fresh).await
    {
        Ok(tools) => Ok(tools),
        Err(error) => agent_fault(ceilings, Some(caller), error, None),
    }
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
