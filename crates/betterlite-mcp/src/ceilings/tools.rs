use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use tracing::warn;

use super::CeilingSource;
use super::Ceilings;
use super::error::{CeilingReadError, CeilingsError, CeilingsResult};
use super::reads::CeilingReads;
use super::servers;
use crate::auth::object_permission_hydration::grants::union_tool_grants;
use crate::auth::object_permission_hydration::reads::ObjectPermission;
use crate::server_grants::ToolGrants;

// the tools a level grants on one server: the union of its expanded direct grants and its toolset grants
pub(super) async fn tool_grants_for_server<Reads>(
    reads: &Reads,
    permission: Option<&ObjectPermission>,
    direct: Option<&ToolGrants>,
    server_id: &str,
    requires_fresh_policy: bool,
) -> Result<Option<Vec<String>>, CeilingReadError<Reads::Error>>
where
    Reads: CeilingReads,
{
    let expanded = reads.expand_tool_permissions(direct);
    let direct_tools = expanded.get(server_id).map(Vec::as_slice);
    let toolset_tools = reads
        .toolset_tools_for_server(permission, server_id, requires_fresh_policy)
        .await?;
    Ok(union_tool_grants(direct_tools, toolset_tools.as_deref()))
}

// LiteLLM's set intersection: a level that grants tools keeps only those the incoming list already allows
pub(super) fn narrowed(
    allowed: Option<Vec<String>>,
    granted: Option<Vec<String>>,
) -> Option<Vec<String>> {
    let Some(granted_tools) = granted else {
        return allowed;
    };
    let Some(allowed_tools) = allowed else {
        return Some(granted_tools);
    };
    Some(servers::distinct(servers::capped(
        allowed_tools,
        &granted_tools,
    )))
}

// the read, its toolset term and the narrowing every human and org level applies; a level with no row leaves the incoming list alone
pub(super) async fn narrow_to_grants<Reads>(
    ceilings: &Ceilings<Reads>,
    server_id: &str,
    auth: Option<&UserAPIKeyAuth>,
    allowed: Option<Vec<String>>,
    permission: Option<ObjectPermission>,
) -> CeilingsResult<Option<Vec<String>>, Reads>
where
    Reads: CeilingReads,
{
    let Some(permission) = permission else {
        return Ok(allowed);
    };
    let fresh = ceilings.reads.requires_fresh_policy(auth);
    let granted = tool_grants_for_server(
        &ceilings.reads,
        Some(&permission),
        permission.mcp_tool_permissions.as_ref(),
        server_id,
        fresh,
    )
    .await?;
    Ok(narrowed(allowed, granted))
}

// the echoed caller team's tool grants on one server; the team read and its toolset read share one fault
pub(super) async fn team_tool_grants<Reads>(
    ceilings: &Ceilings<Reads>,
    caller: &UserAPIKeyAuth,
    server_id: &str,
) -> Result<Option<Vec<String>>, CeilingReadError<Reads::Error>>
where
    Reads: CeilingReads,
{
    let loaded = ceilings.reads.team_object_permission(Some(caller)).await?;
    let direct = loaded
        .as_ref()
        .and_then(|row| row.mcp_tool_permissions.as_ref());
    tool_grants_for_server(
        &ceilings.reads,
        loaded.as_ref(),
        direct,
        server_id,
        ceilings.reads.requires_fresh_policy(Some(caller)),
    )
    .await
}

// an unresolvable org tool ceiling is skipped for a key, and returned for a keyless source or an unloadable row
pub(super) fn org_tools_fault<Reads>(
    tools: Option<Vec<String>>,
    error: CeilingReadError<Reads::Error>,
    source_kind: CeilingSource,
) -> CeilingsResult<Option<Vec<String>>, Reads>
where
    Reads: CeilingReads,
{
    if matches!(source_kind, CeilingSource::Keyless)
        || matches!(error, CeilingReadError::Unloadable(_))
    {
        return Err(CeilingsError::from(error));
    }
    warn!("MCP org tool ceiling unresolvable, the lower-level restrictions stand: {error}");
    Ok(tools)
}
