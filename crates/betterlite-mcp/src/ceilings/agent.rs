use std::error::Error as StdError;
use std::future::Future;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use tracing::warn;

use super::CeilingSource;
use super::Ceilings;
use super::error::{CeilingReadError, CeilingsError, CeilingsResult};
use super::live::decode;
use super::reads::CeilingReads;
use super::servers::{self, ToolsetTerm};
use super::tools::{narrow_to_grants, narrowed, org_tools_fault, tool_grants_for_server};
use crate::auth::object_permission_hydration::reads::ObjectPermission;

// the agent level: its servers and access group ceiling, its per-server grants and the agent and org tool ceilings
pub trait AgentCeilings: Sized {
    type Source: StdError + Send + Sync + 'static;
    fn agent_servers(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        prefetched: Option<&ObjectPermission>,
    ) -> impl Future<Output = Result<Vec<String>, CeilingsError<Self::Source>>> + Send;
    fn agent_access_group_ceiling(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<Option<Vec<String>>, CeilingsError<Self::Source>>> + Send;
    // ast-grep-ignore: no-long-names - get_agent_tool_permissions_for_server reads one agent row's tool grants
    fn agent_tool_permissions_for_server(
        &self,
        server_id: &str,
        auth: Option<&UserAPIKeyAuth>,
        prefetched: Option<&ObjectPermission>,
    ) -> impl Future<Output = Result<Option<Vec<String>>, CeilingsError<Self::Source>>> + Send;
    // ast-grep-ignore: no-long-names - _apply_agent_and_org_tool_ceilings narrows the agent's grants then the org's
    fn apply_agent_and_org_tool_ceilings(
        &self,
        allowed: Option<Vec<String>>,
        server_id: &str,
        auth: Option<&UserAPIKeyAuth>,
        source_kind: CeilingSource,
    ) -> impl Future<Output = Result<Option<Vec<String>>, CeilingsError<Self::Source>>> + Send;
}

impl<Reads: CeilingReads> AgentCeilings for Ceilings<Reads> {
    type Source = Reads::Error;

    // get_allowed_mcp_servers_for_agent: a plain agent reads a union fault as no restriction, a managed one or an unloadable row returns it
    async fn agent_servers(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        prefetched: Option<&ObjectPermission>,
    ) -> CeilingsResult<Vec<String>, Reads> {
        let Some(permission) = agent_permission(self, auth, prefetched).await? else {
            return Ok(Vec::new());
        };
        let fresh = self.reads.requires_fresh_policy(auth);
        let listed =
            servers::permission_servers(&self.reads, &permission, fresh, ToolsetTerm::Counted);
        match listed.await {
            Ok(server_ids) => Ok(server_ids),
            Err(error) => agent_fault(self, auth, error, Vec::new()),
        }
    }

    // _get_agent_access_group_server_ceiling: an attached group set that names no server still denies every server
    async fn agent_access_group_ceiling(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> CeilingsResult<Option<Vec<String>>, Reads> {
        let record = decode::<Reads::Error>(auth)?;
        let Some(agent_id) = record.agent_id else {
            return Ok(None);
        };
        let Some(server_ids) = self.reads.agent_access_group_server_ids(&agent_id).await? else {
            return Ok(None);
        };
        let mut sorted = server_ids;
        sorted.sort();
        let expanded = self.reads.expand_permission_list(&sorted);
        Ok(Some(servers::distinct(expanded)))
    }

    // ast-grep-ignore: no-long-names - direct grants count only when the row lists any, and a plain agent reads a fault as no restriction
    async fn agent_tool_permissions_for_server(
        &self,
        server_id: &str,
        auth: Option<&UserAPIKeyAuth>,
        prefetched: Option<&ObjectPermission>,
    ) -> CeilingsResult<Option<Vec<String>>, Reads> {
        let Some(permission) = agent_permission(self, auth, prefetched).await? else {
            return Ok(None);
        };
        let direct = permission
            .mcp_tool_permissions
            .as_ref()
            .filter(|grants| !grants.is_empty());
        let fresh = self.reads.requires_fresh_policy(auth);
        let granted =
            tool_grants_for_server(&self.reads, Some(&permission), direct, server_id, fresh).await;
        granted.or_else(|error| agent_fault(self, auth, error, None))
    }

    // ast-grep-ignore: no-long-names - the agent's own grants first, then the org's, each only ever narrowing what came before
    async fn apply_agent_and_org_tool_ceilings(
        &self,
        allowed: Option<Vec<String>>,
        server_id: &str,
        auth: Option<&UserAPIKeyAuth>,
        source_kind: CeilingSource,
    ) -> CeilingsResult<Option<Vec<String>>, Reads> {
        let record = decode::<Reads::Error>(auth)?;
        let mut tools = allowed;
        if record.agent_id.is_some() {
            let prefetched = self.reads.agent_object_permission(auth).await?;
            let agent_tools = self
                .agent_tool_permissions_for_server(server_id, auth, prefetched.as_ref())
                .await?;
            tools = narrowed(tools, agent_tools);
        }
        if record.org_id.is_none() {
            return Ok(tools);
        }
        let loaded = match self.reads.org_object_permission(auth).await {
            Ok(loaded) => loaded,
            Err(error) => return org_tools_fault::<Reads>(tools, error, source_kind),
        };
        narrow_to_grants(self, server_id, auth, tools, loaded).await
    }
}

// the agent's own row: no agent or no key reads as nothing, an undecodable key is returned, and a prefetched row wins over a fresh read
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
    let record = decode::<Reads::Error>(Some(caller))?;
    if record.agent_id.is_none() {
        return Ok(None);
    }
    if let Some(permission) = prefetched {
        return Ok(Some(permission.clone()));
    }
    ceilings.reads.agent_object_permission(Some(caller)).await
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
