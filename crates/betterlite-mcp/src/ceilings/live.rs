use std::error::Error as StdError;

use betterlite_auth::agent_access_groups::ceiling::AgentAccessGroups;
use betterlite_auth::agent_access_groups::reads::AccessGroupReads;
use betterlite_auth::agent_caller::caller_auth::agent_caller_auth as caller_auth_record;
use betterlite_auth::agent_policy::admission::managed_agent_policy;
use betterlite_auth::managed_admission::managed_marks;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use super::error::CeilingReadError;
use super::reads::{AdmissionMarks, CeilingReads};
use crate::auth::object_permission_hydration::error::EntitlementError;
use crate::auth::object_permission_hydration::grants::{
    toolset_tool_permissions as toolset_grants, toolset_tools_for_server as toolset_tools_on_server,
};
use crate::auth::object_permission_hydration::principal::{
    NamedPrincipal, named_object_permission, team_object_permission as team_permission_of,
    user_object_permission as user_permission_of,
};
use crate::auth::object_permission_hydration::reads::{ObjectPermission, ObjectPermissionReads};
use crate::auth::object_permission_hydration::record::McpAuth;
use crate::server_grants::auth::GrantAuth;
use crate::server_grants::error::ServerGrantsError;
use crate::server_grants::{GrantReads, ServerGrants, ToolGrants};

// the server manager's registry expansion, sync over the loaded registry
pub trait ExpansionReads: Send + Sync {
    fn expand_permission_list(&self, identifiers: &[String]) -> Vec<String>;

    fn expand_tool_permissions(&self, tool_permissions: Option<&ToolGrants>) -> ToolGrants;
}

// B2314 keeps the two admission marks inside the resolved key, so the admission layer that sets them hands them over
pub trait AdmissionMarkReads: Send + Sync {
    fn admission_marks(&self, auth: &UserAPIKeyAuth) -> AdmissionMarks;
}

// the production port: every read forwards to the owner that already applies its absence and fault rules
pub struct LiveCeilingReads<Grants, Groups, Expansion, Marks>
where
    Grants: GrantReads,
{
    pub grants: ServerGrants<Grants>,
    pub groups: AgentAccessGroups<Groups>,
    pub expansion: Expansion,
    pub marks: Marks,
}

impl<Grants, Groups, Expansion, Marks> CeilingReads
    for LiveCeilingReads<Grants, Groups, Expansion, Marks>
where
    Grants: GrantReads,
    Groups: AccessGroupReads + Send,
    Expansion: ExpansionReads,
    Marks: AdmissionMarkReads,
{
    type Error = <<Grants as GrantReads>::Permissions as ObjectPermissionReads>::Error;

    fn has_database(&self) -> bool {
        self.grants.reads.permissions().has_database()
    }

    fn admission_marks(&self, auth: &UserAPIKeyAuth) -> AdmissionMarks {
        self.marks.admission_marks(auth)
    }

    fn requires_fresh_policy(&self, auth: Option<&UserAPIKeyAuth>) -> bool {
        auth.is_some_and(|caller| managed_marks(caller).requires_fresh_policy)
    }

    async fn org_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> Result<Option<ObjectPermission>, CeilingReadError<Self::Error>> {
        let record = decode(auth)?;
        named_object_permission(
            self.grants.reads.permissions(),
            &record,
            NamedPrincipal::Org,
        )
        .await
        .map_err(entitlement_fault)
    }

    async fn end_user_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> Result<Option<ObjectPermission>, CeilingReadError<Self::Error>> {
        let record = decode(auth)?;
        named_object_permission(
            self.grants.reads.permissions(),
            &record,
            NamedPrincipal::EndUser,
        )
        .await
        .map_err(entitlement_fault)
    }

    async fn user_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> Result<Option<ObjectPermission>, CeilingReadError<Self::Error>> {
        let record = decode(auth)?;
        user_permission_of(self.grants.reads.permissions(), &record)
            .await
            .map_err(|source| CeilingReadError::Failed(source.to_string()))
    }

    async fn agent_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> Result<Option<ObjectPermission>, CeilingReadError<Self::Error>> {
        let record = decode(auth)?;
        named_object_permission(
            self.grants.reads.permissions(),
            &record,
            NamedPrincipal::Agent,
        )
        .await
        .map_err(entitlement_fault)
    }

    async fn team_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> Result<Option<ObjectPermission>, CeilingReadError<Self::Error>> {
        let record = decode(auth)?;
        team_permission_of(self.grants.reads.permissions(), &record)
            .await
            .map_err(|source| CeilingReadError::Failed(source.to_string()))
    }

    async fn servers_from_access_groups(
        &self,
        access_groups: &[String],
        requires_fresh_policy: bool,
    ) -> Result<Vec<String>, CeilingReadError<Self::Error>> {
        self.grants
            .reads
            .servers_from_access_groups(access_groups, requires_fresh_policy)
            .await
            .map_err(|source| CeilingReadError::Failed(source.to_string()))
    }

    async fn team_servers(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> Result<Vec<String>, CeilingReadError<Self::Error>> {
        let Some(caller) = auth else {
            return Ok(Vec::new());
        };
        let grant = GrantAuth::from_auth(caller)
            .map_err(|source| CeilingReadError::Failed(source.to_string()))?;
        self.grants
            .team_servers(Some(&grant))
            .await
            .map_err(grants_fault)
    }

    async fn toolset_tool_permissions(
        &self,
        permission: Option<&ObjectPermission>,
        requires_fresh_policy: bool,
    ) -> Result<ToolGrants, CeilingReadError<Self::Error>> {
        toolset_grants(
            self.grants.reads.permissions(),
            permission,
            requires_fresh_policy,
        )
        .await
        .map_err(entitlement_fault)
    }

    async fn toolset_tools_for_server(
        &self,
        permission: Option<&ObjectPermission>,
        server_id: &str,
        requires_fresh_policy: bool,
    ) -> Result<Option<Vec<String>>, CeilingReadError<Self::Error>> {
        toolset_tools_on_server(
            self.grants.reads.permissions(),
            permission,
            server_id,
            requires_fresh_policy,
        )
        .await
        .map_err(entitlement_fault)
    }

    fn expand_permission_list(&self, identifiers: &[String]) -> Vec<String> {
        self.expansion.expand_permission_list(identifiers)
    }

    fn expand_tool_permissions(&self, tool_permissions: Option<&ToolGrants>) -> ToolGrants {
        self.expansion.expand_tool_permissions(tool_permissions)
    }

    async fn agent_access_group_server_ids(
        &self,
        agent_id: &str,
    ) -> Result<Option<Vec<String>>, CeilingReadError<Self::Error>> {
        self.groups
            .agent_access_group_server_ids(agent_id)
            .await
            .map_err(|source| CeilingReadError::Failed(source.to_string()))
    }

    async fn agent_caller_auth(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> Result<Option<UserAPIKeyAuth>, CeilingReadError<Self::Error>> {
        caller_auth_record(auth).map_err(|source| CeilingReadError::Failed(source.to_string()))
    }

    fn is_managed_agent(&self, auth: Option<&UserAPIKeyAuth>) -> bool {
        managed_agent_policy(auth).is_some()
    }
}

// the caller's wire form decides the ids the getters read, and no auth reads as a record naming nothing
fn decode<Source>(auth: Option<&UserAPIKeyAuth>) -> Result<McpAuth, CeilingReadError<Source>>
where
    Source: StdError + 'static,
{
    match auth {
        Some(caller) => {
            McpAuth::from_auth(caller).map_err(|error| CeilingReadError::Failed(error.to_string()))
        }
        None => Ok(McpAuth::default()),
    }
}

fn entitlement_fault<Source>(error: EntitlementError<Source>) -> CeilingReadError<Source>
where
    Source: StdError + 'static,
{
    match error {
        EntitlementError::Unloadable(unloadable) => CeilingReadError::Unloadable(unloadable),
        EntitlementError::Read { source } => CeilingReadError::Failed(source.to_string()),
    }
}

fn grants_fault<Source>(error: ServerGrantsError<Source>) -> CeilingReadError<Source>
where
    Source: StdError + 'static,
{
    match error {
        ServerGrantsError::Unloadable(unloadable) => CeilingReadError::Unloadable(unloadable),
        error @ (ServerGrantsError::Permission { .. }
        | ServerGrantsError::Read { .. }
        | ServerGrantsError::TeamMissing { .. }) => CeilingReadError::Failed(error.to_string()),
    }
}
