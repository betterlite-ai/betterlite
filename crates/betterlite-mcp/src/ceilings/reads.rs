use std::error::Error as StdError;
use std::future::Future;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use super::error::CeilingReadError;
use crate::auth::object_permission_hydration::reads::ObjectPermission;
use crate::server_grants::ToolGrants;

// the two MCP admission marks betterlite-auth keeps inside the resolved key, which no public view exposes
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AdmissionMarks {
    pub admitted_user_subject: bool,
    pub explicit_grants_only: bool,
}

// Every read a ceiling makes, with each owner's own rules for absent rows and faults already applied.
pub trait CeilingReads: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn has_database(&self) -> bool;

    // B2314 keeps the admission marks and the fresh-policy flag inside the resolved key, so the port is where admission and the managed marks hand them over
    fn admission_marks(&self, auth: &UserAPIKeyAuth) -> AdmissionMarks;

    fn requires_fresh_policy(&self, auth: Option<&UserAPIKeyAuth>) -> bool;

    // B4668's getters for the principal that names an object_permission_id, each applying its absences and its unreadable rows
    fn org_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<Option<ObjectPermission>, CeilingReadError<Self::Error>>> + Send;

    fn end_user_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<Option<ObjectPermission>, CeilingReadError<Self::Error>>> + Send;

    fn user_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<Option<ObjectPermission>, CeilingReadError<Self::Error>>> + Send;

    fn agent_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<Option<ObjectPermission>, CeilingReadError<Self::Error>>> + Send;

    fn team_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<Option<ObjectPermission>, CeilingReadError<Self::Error>>> + Send;

    // B10078's servers_from_access_groups and B4669's team_servers, which own their own width of the fault rules
    fn servers_from_access_groups(
        &self,
        access_groups: &[String],
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<Vec<String>, CeilingReadError<Self::Error>>> + Send;

    fn team_servers(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<Vec<String>, CeilingReadError<Self::Error>>> + Send;

    // B4668's toolset reads: a declared toolset that resolves to nothing is an unloadable entitlement
    fn toolset_tool_permissions(
        &self,
        permission: Option<&ObjectPermission>,
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<ToolGrants, CeilingReadError<Self::Error>>> + Send;

    fn toolset_tools_for_server(
        &self,
        permission: Option<&ObjectPermission>,
        server_id: &str,
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<Option<Vec<String>>, CeilingReadError<Self::Error>>> + Send;

    // the server manager's registry expansion, sync because it reads the loaded registry and no owner exists at the pin
    fn expand_permission_list(&self, identifiers: &[String]) -> Vec<String>;

    fn expand_tool_permissions(&self, tool_permissions: Option<&ToolGrants>) -> ToolGrants;

    // B4778's resolve_agent_access_group_ceiling: None only when the agent attaches no group
    fn agent_access_group_server_ids(
        &self,
        agent_id: &str,
    ) -> impl Future<Output = Result<Option<Vec<String>>, CeilingReadError<Self::Error>>> + Send;

    // B4778's agent_caller_auth: a bare record holding the echoed user_id and team_id, None when nothing was echoed
    fn agent_caller_auth(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> impl Future<Output = Result<Option<UserAPIKeyAuth>, CeilingReadError<Self::Error>>> + Send;

    // B4772's managed_agent_policy is present on the auth
    fn is_managed_agent(&self, auth: Option<&UserAPIKeyAuth>) -> bool;
}
