#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, Ready, ready};
use std::sync::Mutex;

use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_db::budget_permission_rows::PermissionList;
use indexmap::IndexSet;
use thiserror::Error;

use super::auth::GrantAuth;
use super::error::GrantReadError;
use super::{GrantReads, RegistryGrantReads, ServerGrants, TeamRecord, ToolGrants};
use crate::auth::object_permission_hydration::reads::{
    CachedLink, EndUserEntitlement, ObjectPermission, ObjectPermissionReads, OrgPermissionLink,
};
use crate::auth::object_permission_hydration::record::McpAuth;

mod db_reads_tests;
mod key_tests;
mod team_error_tests;
mod team_tests;
mod user_teams_tests;

#[derive(Debug, Error)]
#[error("the store is down")]
struct StoreDown;

#[derive(Clone, Debug)]
enum Scripted<T> {
    Value(T),
    Fault,
}

struct FakeReads {
    database: bool,
    registry_ids: Vec<String>,
    // a name or alias and the server ids it expands to, the way the registry matches them
    aliases: BTreeMap<String, Vec<String>>,
    permission: Scripted<Option<ObjectPermission>>,
    toolsets: Scripted<ToolGrants>,
    team: Scripted<Option<TeamRecord>>,
    user_teams: Scripted<Option<Vec<String>>>,
    unified: Scripted<Vec<String>>,
    legacy: Scripted<Vec<String>>,
    log: Mutex<Vec<String>>,
}

impl FakeReads {
    fn standard() -> Self {
        Self {
            database: true,
            registry_ids: names(&["srv-gh", "srv-direct", "srv-group", "srv-tools"]),
            aliases: BTreeMap::from([
                (String::from("github"), names(&["srv-gh"])),
                (String::from("tool-alias"), names(&["srv-tools"])),
            ]),
            permission: Scripted::Value(None),
            toolsets: Scripted::Value(BTreeMap::new()),
            team: Scripted::Value(None),
            user_teams: Scripted::Value(None),
            unified: Scripted::Value(Vec::new()),
            legacy: Scripted::Value(Vec::new()),
            log: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, entry: String) {
        self.log.lock().unwrap().push(entry);
    }

    fn calls(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn grant_read<T: Clone>(
        &self,
        label: &str,
        inputs: &dyn Debug,
        fresh: bool,
        script: &Scripted<T>,
    ) -> Ready<Result<T, GrantReadError>> {
        self.record(format!("{label}:{inputs:?}:{fresh}"));
        match script {
            Scripted::Value(value) => ready(Ok(value.clone())),
            Scripted::Fault => ready(Err(GrantReadError::Team(TeamLookupError::NoDatabase))),
        }
    }

    fn stored<T: Clone>(&self, entry: String, script: &Scripted<T>) -> Result<T, StoreDown> {
        self.record(entry);
        match script {
            Scripted::Value(value) => Ok(value.clone()),
            Scripted::Fault => Err(StoreDown),
        }
    }

    fn expand(&self, identifier: &str) -> Vec<String> {
        if self.registry_ids.iter().any(|id| id.as_str() == identifier) {
            return Vec::from([identifier.to_owned()]);
        }
        self.aliases
            .get(identifier)
            .cloned()
            .unwrap_or_else(|| Vec::from([identifier.to_owned()]))
    }
}

impl RegistryGrantReads for FakeReads {
    fn expand_permission_list(
        &self,
        identifiers: &[String],
    ) -> impl Future<Output = Vec<String>> + Send {
        self.record(format!("expand_permission_list:{identifiers:?}"));
        let expanded: IndexSet<String> = identifiers
            .iter()
            .flat_map(|identifier| self.expand(identifier))
            .collect();
        ready(expanded.into_iter().collect())
    }

    fn expand_tool_permissions(
        &self,
        tool_permissions: Option<&ToolGrants>,
    ) -> impl Future<Output = ToolGrants> + Send {
        self.record(format!("expand_tool_permissions:{tool_permissions:?}"));
        let mut expanded = ToolGrants::new();
        for (key, tools) in tool_permissions.into_iter().flatten() {
            for server_id in self.expand(key) {
                expanded.entry(server_id).or_default().extend(tools.clone());
            }
        }
        ready(expanded)
    }

    fn registry_server_ids(&self) -> impl Future<Output = Vec<String>> + Send {
        self.record(String::from("registry_server_ids"));
        ready(self.registry_ids.clone())
    }
}

impl GrantReads for FakeReads {
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
        team_id: &str,
        fresh: bool,
    ) -> impl Future<Output = Result<Option<TeamRecord>, GrantReadError>> + Send {
        self.grant_read("team", team_id, fresh, &self.team)
    }

    fn user_team_ids(
        &self,
        user_id: &str,
        fresh: bool,
    ) -> impl Future<Output = Result<Option<Vec<String>>, GrantReadError>> + Send {
        self.grant_read("user_team_ids", user_id, fresh, &self.user_teams)
    }

    fn access_group_server_ids(
        &self,
        ids: &[String],
        fresh: bool,
    ) -> impl Future<Output = Result<Vec<String>, GrantReadError>> + Send {
        self.grant_read("access_group_server_ids", ids, fresh, &self.unified)
    }

    fn servers_from_access_groups(
        &self,
        groups: &[String],
        fresh: bool,
    ) -> impl Future<Output = Result<Vec<String>, GrantReadError>> + Send {
        self.grant_read("servers_from_access_groups", groups, fresh, &self.legacy)
    }
}

impl ObjectPermissionReads for FakeReads {
    type Error = StoreDown;

    fn has_database(&self) -> bool {
        self.database
    }

    fn management_ttl(&self) -> f64 {
        0.0_f64
    }

    fn cached_link(
        &self,
        _key: &str,
    ) -> impl Future<Output = Result<CachedLink, Self::Error>> + Send {
        ready(Err(StoreDown))
    }

    fn store_link(
        &self,
        _key: &str,
        _value: &str,
        _ttl: f64,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        ready(Err(StoreDown))
    }

    fn permission(
        &self,
        object_permission_id: &str,
        check_db_only: bool,
    ) -> impl Future<Output = Result<Option<ObjectPermission>, Self::Error>> + Send {
        let entry = format!("permission:{object_permission_id}:{check_db_only}");
        ready(self.stored(entry, &self.permission))
    }

    fn team_permission(
        &self,
        _team_id: &str,
        _check_db_only: bool,
    ) -> impl Future<Output = Result<Option<ObjectPermission>, Self::Error>> + Send {
        ready(Err(StoreDown))
    }

    fn org_object_permission_link(
        &self,
        _org_id: &str,
    ) -> impl Future<Output = Result<OrgPermissionLink, Self::Error>> + Send {
        ready(Err(StoreDown))
    }

    fn end_user_entitlement(
        &self,
        _end_user_id: &str,
    ) -> impl Future<Output = Result<Option<EndUserEntitlement>, Self::Error>> + Send {
        ready(Err(StoreDown))
    }

    fn user_object_permission_link(
        &self,
        _user_id: &str,
        _check_db_only: bool,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> + Send {
        ready(Err(StoreDown))
    }

    fn agent_object_permission_link(
        &self,
        _agent_id: &str,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> + Send {
        ready(Err(StoreDown))
    }

    fn resolve_toolset_grants(
        &self,
        toolset_ids: &[String],
        check_db_only: bool,
    ) -> impl Future<Output = Result<ToolGrants, Self::Error>> + Send {
        let entry = format!("resolve_toolset_grants:{toolset_ids:?}:{check_db_only}");
        ready(self.stored(entry, &self.toolsets))
    }
}

fn names(entries: &[&str]) -> Vec<String> {
    entries.iter().map(|entry| String::from(*entry)).collect()
}

fn sorted(mut entries: Vec<String>) -> Vec<String> {
    entries.sort();
    entries
}

fn grants(reads: FakeReads) -> ServerGrants<FakeReads> {
    ServerGrants { reads }
}

fn row() -> ObjectPermission {
    ObjectPermission {
        object_permission_id: String::from("op-1"),
        mcp_servers: PermissionList(None),
        mcp_access_groups: PermissionList(None),
        mcp_tool_permissions: None,
        vector_stores: PermissionList(None),
        agents: PermissionList(None),
        agent_access_groups: PermissionList(None),
        models: PermissionList(None),
        mcp_toolsets: None,
        blocked_tools: PermissionList(None),
        search_tools: PermissionList(None),
        mcp_tool_search_enabled: None,
        skills: None,
    }
}

// every source of a grant filled in: a name, an id, an unknown entry, a group, a tool alias and a toolset
fn four_source_row() -> ObjectPermission {
    let mut permission = row();
    permission.mcp_servers = PermissionList(Some(names(&["github", "srv-direct", "unknown-name"])));
    permission.mcp_access_groups = PermissionList(Some(names(&["grp-1"])));
    permission.mcp_tool_permissions = Some(BTreeMap::from([(
        String::from("tool-alias"),
        names(&["read"]),
    )]));
    permission.mcp_toolsets = Some(names(&["ts-1"]));
    permission
}

fn toolset_grants() -> ToolGrants {
    BTreeMap::from([(String::from("srv-toolset"), names(&["list"]))])
}

fn grant_auth(team_id: Option<&str>, requires_fresh_policy: bool) -> GrantAuth {
    GrantAuth {
        mcp: McpAuth {
            user_id: Some(String::from("user-1")),
            team_id: team_id.map(String::from),
            org_id: None,
            end_user_id: None,
            agent_id: None,
            object_permission_id: None,
            object_permission: None,
            requires_fresh_policy,
        },
        access_group_ids: Vec::new(),
    }
}

fn key_auth(permission: ObjectPermission, requires_fresh_policy: bool) -> GrantAuth {
    let mut auth = grant_auth(None, requires_fresh_policy);
    auth.mcp.object_permission = Some(permission);
    auth
}

fn team_record(permission: Option<ObjectPermission>) -> TeamRecord {
    TeamRecord {
        blocked: false,
        access_group_ids: names(&["ag-team"]),
        object_permission: permission,
    }
}
