use std::collections::BTreeMap;
use std::future::Future;

use crate::auth::object_permission_hydration::reads::{ObjectPermission, ObjectPermissionReads};

use self::error::{GrantReadError, ServerGrantsError};

pub mod auth;
pub mod db_reads;
pub mod error;
pub mod key;
pub mod team;
pub mod user;

mod tests;

// SpecialMCPServerNames.no_mcp_servers: a key that lists it opts out of every MCP server
pub const NO_MCP_SERVERS: &str = "no-mcp-servers";
// SpecialMCPServerName.all_proxy_servers: special only on a team, where it grants the whole registry
pub const ALL_PROXY_MCP_SERVERS: &str = "all-proxy-mcpservers";

pub type ToolGrants = BTreeMap<String, Vec<String>>;

pub type GrantsError<Reads> =
    ServerGrantsError<<<Reads as GrantReads>::Permissions as ObjectPermissionReads>::Error>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamRecord {
    pub blocked: bool,
    pub access_group_ids: Vec<String>,
    pub object_permission: Option<ObjectPermission>,
}

// the server manager's registry expansion; its own task implements this over the live registry
pub trait RegistryGrantReads: Send + Sync {
    // names and aliases become every matching server id, an id or an unknown entry passes through
    fn expand_permission_list(
        &self,
        identifiers: &[String],
    ) -> impl Future<Output = Vec<String>> + Send;

    fn expand_tool_permissions(
        &self,
        tool_permissions: Option<&ToolGrants>,
    ) -> impl Future<Output = ToolGrants> + Send;

    fn registry_server_ids(&self) -> impl Future<Output = Vec<String>> + Send;
}

pub trait GrantReads: Send + Sync {
    type Permissions: ObjectPermissionReads;
    type Registry: RegistryGrantReads;

    // B4668's reads: the hydrated key permission, has_database and the toolset resolver
    fn permissions(&self) -> &Self::Permissions;

    fn registry(&self) -> &Self::Registry;

    // get_team_object: None is a team row the database does not have
    fn team(
        &self,
        team_id: &str,
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<Option<TeamRecord>, GrantReadError>> + Send;

    // get_user_object without upsert: None is a user the database does not have
    fn user_team_ids(
        &self,
        user_id: &str,
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<Option<Vec<String>>, GrantReadError>> + Send;

    // _get_mcp_server_ids_from_access_groups over the unified access groups
    fn access_group_server_ids(
        &self,
        access_group_ids: &[String],
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<Vec<String>, GrantReadError>> + Send;

    // B10078's servers_from_access_groups over the tag-style mcp_access_groups
    fn servers_from_access_groups(
        &self,
        access_groups: &[String],
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<Vec<String>, GrantReadError>> + Send;
}

pub struct ServerGrants<Reads: GrantReads> {
    pub reads: Reads,
}
