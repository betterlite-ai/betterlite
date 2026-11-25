use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use tracing::warn;

use super::CeilingSource;
use super::Ceilings;
use super::agent::{AgentServer, agent_server_tools};
use super::error::{CeilingReadError, CeilingsError, CeilingsResult};
use super::reads::CeilingReads;
use super::servers;
use crate::auth::object_permission_hydration::grants::union_tool_grants;
use crate::auth::object_permission_hydration::reads::ObjectPermission;
use crate::auth::object_permission_hydration::record::McpAuth;
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
fn narrowed(allowed: Option<Vec<String>>, granted: Option<Vec<String>>) -> Option<Vec<String>> {
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

// one tool ceiling call: the tools a level narrows, the server they sit on and the caller they are narrowed for
pub(super) struct ToolCall<'caller> {
    pub(super) allowed: Option<Vec<String>>,
    pub(super) server_id: &'caller str,
    pub(super) auth: Option<&'caller UserAPIKeyAuth>,
}

// the read, its toolset term and the narrowing every human and org level applies once it holds a permission
async fn narrow_to_grants<Reads>(
    ceilings: &Ceilings<Reads>,
    server_id: &str,
    auth: Option<&UserAPIKeyAuth>,
    allowed: Option<Vec<String>>,
    permission: &ObjectPermission,
) -> CeilingsResult<Option<Vec<String>>, Reads>
where
    Reads: CeilingReads,
{
    let fresh = ceilings.reads.requires_fresh_policy(auth);
    let granted = tool_grants_for_server(
        &ceilings.reads,
        Some(permission),
        permission.mcp_tool_permissions.as_ref(),
        server_id,
        fresh,
    )
    .await?;
    Ok(narrowed(allowed, granted))
}

// the two human tool levels: whose row is read, and whether a keyless source is left alone
pub(super) enum ToolLevel {
    User(CeilingSource),
    EndUser,
}

// _apply_user_tool_ceiling and _apply_end_user_tool_ceiling: a keyless source or an end user with nothing to read keeps every tool; a user fault denies every tool and an end-user fault is returned
pub(super) async fn apply_human_tool_ceiling<Reads>(
    ceilings: &Ceilings<Reads>,
    call: ToolCall<'_>,
    level: ToolLevel,
) -> CeilingsResult<Option<Vec<String>>, Reads>
where
    Reads: CeilingReads,
{
    let loaded = match level {
        ToolLevel::User(source_kind) => {
            if matches!(source_kind, CeilingSource::Keyless) {
                return Ok(call.allowed);
            }
            match ceilings.reads.user_object_permission(call.auth).await {
                Ok(loaded) => loaded,
                Err(error) => {
                    warn!(
                        "MCP user tool ceiling unresolvable, denying tools on {:?}: {error}",
                        call.server_id
                    );
                    return Ok(Some(Vec::new()));
                }
            }
        }
        ToolLevel::EndUser => {
            let Some(record) = call.auth.and_then(|caller| McpAuth::from_auth(caller).ok()) else {
                return Ok(call.allowed);
            };
            if record.end_user_id.is_none() || !ceilings.reads.has_database() {
                return Ok(call.allowed);
            }
            ceilings.reads.end_user_object_permission(call.auth).await?
        }
    };
    let Some(permission) = loaded else {
        return Ok(call.allowed);
    };
    narrow_to_grants(
        ceilings,
        call.server_id,
        call.auth,
        call.allowed,
        &permission,
    )
    .await
}

// apply_agent_caller_tool_ceiling: a caller team that cannot be read denies every tool, then the echoed user's own tool ceiling
pub(super) async fn apply_caller_tools<Reads>(
    ceilings: &Ceilings<Reads>,
    call: ToolCall<'_>,
) -> CeilingsResult<Option<Vec<String>>, Reads>
where
    Reads: CeilingReads,
{
    let ToolCall {
        allowed,
        server_id,
        auth,
    } = call;
    let Some(source) = auth else {
        return Ok(allowed);
    };
    let Some(caller) = ceilings.reads.agent_caller_auth(source).await? else {
        return Ok(allowed);
    };
    let team = match team_tool_grants(ceilings, &caller, server_id).await {
        Ok(team) => team,
        Err(error) => {
            warn!("MCP agent caller team tool ceiling unresolvable on {server_id:?}: {error}");
            return Ok(Some(Vec::new()));
        }
    };
    let team_capped = narrowed(allowed, team);
    let next = ToolCall {
        allowed: team_capped,
        server_id,
        auth: Some(&caller),
    };
    apply_human_tool_ceiling(ceilings, next, ToolLevel::User(CeilingSource::Key)).await
}

// _apply_agent_and_org_tool_ceilings: the agent's own grants first, then the org's, each only ever narrowing what came before
pub(super) async fn apply_agent_org_tools<Reads>(
    ceilings: &Ceilings<Reads>,
    call: ToolCall<'_>,
    source_kind: CeilingSource,
) -> CeilingsResult<Option<Vec<String>>, Reads>
where
    Reads: CeilingReads,
{
    let ToolCall {
        allowed,
        server_id,
        auth,
    } = call;
    let Some(record) = auth.and_then(|caller| McpAuth::from_auth(caller).ok()) else {
        return Ok(allowed);
    };
    let mut tools = allowed;
    if record.agent_id.is_some() {
        let prefetched = ceilings.reads.agent_object_permission(auth).await?;
        let server = AgentServer {
            server_id,
            auth,
            prefetched: prefetched.as_ref(),
        };
        let agent_tools = agent_server_tools(ceilings, server).await?;
        tools = narrowed(tools, agent_tools);
    }
    if record.org_id.is_none() {
        return Ok(tools);
    }
    let loaded = match ceilings.reads.org_object_permission(auth).await {
        Ok(loaded) => loaded,
        Err(error) => return org_tools_fault::<Reads>(tools, error, source_kind),
    };
    let Some(permission) = loaded else {
        return Ok(tools);
    };
    narrow_to_grants(ceilings, server_id, auth, tools, &permission).await
}

// the echoed caller team's tool grants on one server; the team read and its toolset read share one fault
async fn team_tool_grants<Reads>(
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
fn org_tools_fault<Reads>(
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
