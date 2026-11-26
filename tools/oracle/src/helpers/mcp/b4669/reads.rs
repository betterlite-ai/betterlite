use std::future::{Future, ready};

use betterlite_db::mcp::table_access::store::McpTableStore;
use betterlite_mcp::access_group_servers::servers_from_access_groups;
use betterlite_mcp::auth::object_permission_hydration::reads::{
    CachedLink, EndUserEntitlement, ObjectPermission, ObjectPermissionReads, OrgPermissionLink,
};
use betterlite_mcp::registry::servers::{ServerRegistry, SharedRegistry};
use betterlite_mcp::server_grants::error::GrantReadError;
use betterlite_mcp::server_grants::{GrantReads, RegistryGrantReads, TeamRecord, ToolGrants};
use thiserror::Error;

use super::AccessGroupRecord;

#[derive(Debug, Error)]
#[error("this read is not part of the case")]
pub(crate) struct Unscripted;

// the case's rows in place of the database; the key permission comes loaded, so the row reads are never asked
pub(crate) struct CaseReads {
    pub(crate) registry: SharedRegistry,
    pub(crate) access_groups: Vec<AccessGroupRecord>,
    pub(crate) toolset_grants: ToolGrants,
    pub(crate) team: Option<TeamRecord>,
    pub(crate) user_teams: Option<Vec<String>>,
}

// LiteLLM's expand_permission_list: an id passes, a name, alias or server_name becomes every match, the rest passes
fn expand(registry: &ServerRegistry, identifier: &str) -> Vec<String> {
    let servers = registry.get_registry();
    if servers
        .iter()
        .any(|server| server.fields.server_id == identifier)
    {
        return Vec::from([identifier.to_owned()]);
    }
    let matches: Vec<String> = servers
        .iter()
        .filter(|server| {
            server.fields.alias.as_deref() == Some(identifier)
                || server.fields.server_name.as_deref() == Some(identifier)
                || server.fields.name == identifier
        })
        .map(|server| server.fields.server_id.clone())
        .collect();
    if matches.is_empty() {
        return Vec::from([identifier.to_owned()]);
    }
    matches
}

impl RegistryGrantReads for CaseReads {
    async fn expand_permission_list(&self, identifiers: &[String]) -> Vec<String> {
        let registry = self.registry.read().await;
        let mut expanded = Vec::new();
        for server_id in identifiers
            .iter()
            .flat_map(|identifier| expand(&registry, identifier))
        {
            if !expanded.contains(&server_id) {
                expanded.push(server_id);
            }
        }
        expanded
    }

    async fn expand_tool_permissions(&self, tool_permissions: Option<&ToolGrants>) -> ToolGrants {
        let registry = self.registry.read().await;
        let mut expanded = ToolGrants::new();
        for (key, tools) in tool_permissions.into_iter().flatten() {
            for server_id in expand(&registry, key) {
                let granted = expanded.entry(server_id).or_default();
                for tool in tools {
                    if !granted.contains(tool) {
                        granted.push(tool.clone());
                    }
                }
            }
        }
        expanded
    }

    async fn registry_server_ids(&self) -> Vec<String> {
        let registry = self.registry.read().await;
        registry
            .get_registry()
            .into_iter()
            .map(|server| server.fields.server_id.clone())
            .collect()
    }
}

impl GrantReads for CaseReads {
    type Permissions = Self;
    type Registry = Self;

    fn permissions(&self) -> &Self {
        self
    }

    fn registry(&self) -> &Self {
        self
    }

    fn team(
        &self,
        _team_id: &str,
        _requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<Option<TeamRecord>, GrantReadError>> + Send {
        ready(Ok(self.team.clone()))
    }

    fn user_team_ids(
        &self,
        _user_id: &str,
        _requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<Option<Vec<String>>, GrantReadError>> + Send {
        ready(Ok(self.user_teams.clone()))
    }

    // _get_resources_from_access_groups: a group the rows do not hold adds nothing
    fn access_group_server_ids(
        &self,
        access_group_ids: &[String],
        _requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<Vec<String>, GrantReadError>> + Send {
        let mut server_ids: Vec<String> = Vec::new();
        for group in &self.access_groups {
            if !access_group_ids.contains(&group.access_group_id) {
                continue;
            }
            for server_id in &group.access_mcp_server_ids {
                if !server_ids.contains(server_id) {
                    server_ids.push(server_id.clone());
                }
            }
        }
        ready(Ok(server_ids))
    }

    // the case has no database servers, which is B10078's read with no rows port
    async fn servers_from_access_groups(
        &self,
        access_groups: &[String],
        requires_fresh_policy: bool,
    ) -> Result<Vec<String>, GrantReadError> {
        servers_from_access_groups(
            &self.registry,
            None::<&McpTableStore<'_>>,
            access_groups,
            requires_fresh_policy,
        )
        .await
        .map_err(|error| GrantReadError::AccessGroupServers(Box::new(error)))
    }
}

impl ObjectPermissionReads for CaseReads {
    type Error = Unscripted;

    fn has_database(&self) -> bool {
        true
    }

    fn management_ttl(&self) -> f64 {
        0.0_f64
    }

    fn cached_link(
        &self,
        _key: &str,
    ) -> impl Future<Output = Result<CachedLink, Self::Error>> + Send {
        ready(Err(Unscripted))
    }

    fn store_link(
        &self,
        _key: &str,
        _value: &str,
        _ttl: f64,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        ready(Err(Unscripted))
    }

    fn permission(
        &self,
        _object_permission_id: &str,
        _check_db_only: bool,
    ) -> impl Future<Output = Result<Option<ObjectPermission>, Self::Error>> + Send {
        ready(Err(Unscripted))
    }

    fn team_permission(
        &self,
        _team_id: &str,
        _check_db_only: bool,
    ) -> impl Future<Output = Result<Option<ObjectPermission>, Self::Error>> + Send {
        ready(Err(Unscripted))
    }

    fn org_object_permission_link(
        &self,
        _org_id: &str,
    ) -> impl Future<Output = Result<OrgPermissionLink, Self::Error>> + Send {
        ready(Err(Unscripted))
    }

    fn end_user_entitlement(
        &self,
        _end_user_id: &str,
    ) -> impl Future<Output = Result<Option<EndUserEntitlement>, Self::Error>> + Send {
        ready(Err(Unscripted))
    }

    fn user_object_permission_link(
        &self,
        _user_id: &str,
        _check_db_only: bool,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> + Send {
        ready(Err(Unscripted))
    }

    fn agent_object_permission_link(
        &self,
        _agent_id: &str,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> + Send {
        ready(Err(Unscripted))
    }

    // the case seeds one answer for the declared toolsets, as the toolset cache would hold it
    fn resolve_toolset_grants(
        &self,
        _toolset_ids: &[String],
        _check_db_only: bool,
    ) -> impl Future<Output = Result<ToolGrants, Self::Error>> + Send {
        ready(Ok(self.toolset_grants.clone()))
    }
}
