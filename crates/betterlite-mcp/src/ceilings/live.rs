use std::error::Error as StdError;
use std::future::{Future, ready};

use betterlite_auth::agent_access_groups::ceiling::AgentAccessGroups;
use betterlite_auth::agent_access_groups::reads::AccessGroupReads;
use betterlite_auth::agent_caller::caller_auth::agent_caller_auth as caller_auth_record;
use betterlite_auth::agent_policy::admission::managed_agent_policy;
use betterlite_auth::auth_cache::copy::copy_for_cache;
use betterlite_auth::managed_admission::managed_marks;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use super::error::CeilingReadError;
use super::reads::{AdmissionMarks, CeilingReads};
use crate::auth::object_permission_hydration::grants::toolset_tool_permissions as toolset_grants;
use crate::auth::object_permission_hydration::principal::{
    NamedPrincipal, named_object_permission, team_object_permission as team_permission_of,
    user_object_permission as user_permission_of,
};
use crate::auth::object_permission_hydration::reads::{ObjectPermission, ObjectPermissionReads};
use crate::auth::object_permission_hydration::record::McpAuth;
use crate::server_grants::auth::GrantAuth;
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
        read_permission(self.grants.reads.permissions(), auth, Principal::Org).await
    }

    async fn end_user_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> Result<Option<ObjectPermission>, CeilingReadError<Self::Error>> {
        read_permission(self.grants.reads.permissions(), auth, Principal::EndUser).await
    }

    async fn user_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> Result<Option<ObjectPermission>, CeilingReadError<Self::Error>> {
        read_permission(self.grants.reads.permissions(), auth, Principal::User).await
    }

    async fn agent_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> Result<Option<ObjectPermission>, CeilingReadError<Self::Error>> {
        read_permission(self.grants.reads.permissions(), auth, Principal::Agent).await
    }

    async fn team_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> Result<Option<ObjectPermission>, CeilingReadError<Self::Error>> {
        read_permission(self.grants.reads.permissions(), auth, Principal::Team).await
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
        let grant = GrantAuth::from_auth(&copy_for_cache(caller))
            .map_err(|source| CeilingReadError::Failed(source.to_string()))?;
        self.grants
            .team_servers(Some(&grant))
            .await
            .map_err(CeilingReadError::from)
    }

    async fn toolset_tool_permissions(
        &self,
        permission: Option<&ObjectPermission>,
        requires_fresh_policy: bool,
    ) -> Result<ToolGrants, CeilingReadError<Self::Error>> {
        let reads = self.grants.reads.permissions();
        toolset_grants(reads, permission, requires_fresh_policy)
            .await
            .map_err(CeilingReadError::from)
    }

    // LiteLLM's get_toolset_tools_for_server is get_toolset_tool_permissions(...)[server_id], so the read happens once here
    async fn toolset_tools_for_server(
        &self,
        permission: Option<&ObjectPermission>,
        server_id: &str,
        requires_fresh_policy: bool,
    ) -> Result<Option<Vec<String>>, CeilingReadError<Self::Error>> {
        let mut grants = self
            .toolset_tool_permissions(permission, requires_fresh_policy)
            .await?;
        Ok(grants.remove(server_id))
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

    fn agent_caller_auth(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> impl Future<Output = Result<Option<UserAPIKeyAuth>, CeilingReadError<Self::Error>>> + Send
    {
        ready(
            caller_auth_record(auth).map_err(|source| CeilingReadError::Failed(source.to_string())),
        )
    }

    fn is_managed_agent(&self, auth: Option<&UserAPIKeyAuth>) -> bool {
        managed_agent_policy(auth).is_some()
    }
}

// the caller's wire form decides the ids the getters read; request tracing data is no part of it, and no auth reads as a record naming nothing
pub(super) fn decode<Source>(
    auth: Option<&UserAPIKeyAuth>,
) -> Result<McpAuth, CeilingReadError<Source>>
where
    Source: StdError + 'static,
{
    auth.map_or_else(
        || Ok(McpAuth::default()),
        |caller| {
            McpAuth::from_auth(&copy_for_cache(caller))
                .map_err(|error| CeilingReadError::Failed(error.to_string()))
        },
    )
}

// which principal a getter reads, so the five getters share one read and one fault mapping
enum Principal {
    Org,
    EndUser,
    Agent,
    User,
    Team,
}

async fn read_permission<Reads>(
    reads: &Reads,
    auth: Option<&UserAPIKeyAuth>,
    which: Principal,
) -> Result<Option<ObjectPermission>, CeilingReadError<Reads::Error>>
where
    Reads: ObjectPermissionReads,
{
    let record = decode(auth)?;
    match which {
        Principal::Org => named_object_permission(reads, &record, NamedPrincipal::Org)
            .await
            .map_err(CeilingReadError::from),
        Principal::EndUser => named_object_permission(reads, &record, NamedPrincipal::EndUser)
            .await
            .map_err(CeilingReadError::from),
        Principal::Agent => named_object_permission(reads, &record, NamedPrincipal::Agent)
            .await
            .map_err(CeilingReadError::from),
        Principal::User => user_permission_of(reads, &record)
            .await
            .map_err(|source| CeilingReadError::Failed(source.to_string())),
        Principal::Team => team_permission_of(reads, &record)
            .await
            .map_err(|source| CeilingReadError::Failed(source.to_string())),
    }
}
