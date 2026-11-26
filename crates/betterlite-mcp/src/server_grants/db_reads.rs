use betterlite_auth::auth_team_lookup::access::get_access_object;
use betterlite_auth::auth_team_lookup::lookup::{TeamLookup, TeamRequest, TeamSource};
use betterlite_auth::auth_team_lookup::team::get_team_object;
use betterlite_auth::auth_user_lookup::lookup::{UserLookup, UserLookupRequest};
use betterlite_db::mcp::table_access::store::McpTableStore;
use indexmap::IndexSet;
use tracing::debug;

use super::error::GrantReadError;
use super::{GrantReads, RegistryGrantReads, TeamRecord};
use crate::access_group_servers::servers_from_access_groups;
use crate::auth::object_permission_hydration::reads::ObjectPermissionReads;
use crate::registry::servers::SharedRegistry;

// B737's team and access group reads, B736's user read and B10078's servers; permissions and registry come from their owners
pub struct DbGrantReads<'db, Permissions, Registry> {
    pub permissions: Permissions,
    pub registry: Registry,
    pub teams: &'db TeamLookup,
    pub users: &'db UserLookup,
    pub servers: &'db SharedRegistry,
    pub server_rows: Option<McpTableStore<'db>>,
}

impl<Permissions, Registry> GrantReads for DbGrantReads<'_, Permissions, Registry>
where
    Permissions: ObjectPermissionReads,
    Registry: RegistryGrantReads,
{
    type Permissions = Permissions;
    type Registry = Registry;

    fn permissions(&self) -> &Permissions {
        &self.permissions
    }

    fn registry(&self) -> &Registry {
        &self.registry
    }

    // get_team_object raises for a missing row, so the lookup's not-found stays an error
    async fn team(
        &self,
        team_id: &str,
        requires_fresh_policy: bool,
    ) -> Result<Option<TeamRecord>, GrantReadError> {
        let source = if requires_fresh_policy {
            TeamSource::Writer
        } else {
            TeamSource::CacheThenDatabase
        };
        let request = TeamRequest {
            team_id,
            source,
            team_id_upsert: false,
        };
        let cached = get_team_object(self.teams, request)
            .await
            .map_err(GrantReadError::Team)?;
        let fields = cached.team.fields;
        Ok(Some(TeamRecord {
            blocked: fields.base.blocked,
            access_group_ids: fields.base.access_group_ids.unwrap_or_default(),
            object_permission: fields.object_permission,
        }))
    }

    async fn user_team_ids(
        &self,
        user_id: &str,
        requires_fresh_policy: bool,
    ) -> Result<Option<Vec<String>>, GrantReadError> {
        let request = UserLookupRequest {
            user_id: Some(user_id),
            sso_user_id: None,
            user_email: None,
            user_id_upsert: false,
            check_db_only: Some(requires_fresh_policy),
        };
        let found = self
            .users
            .get_user_object(request)
            .await
            .map_err(GrantReadError::User)?;
        Ok(found.map(|user| user.teams))
    }

    // _get_resources_from_access_groups: a group that does not load adds nothing unless the policy must be fresh
    async fn access_group_server_ids(
        &self,
        access_group_ids: &[String],
        requires_fresh_policy: bool,
    ) -> Result<Vec<String>, GrantReadError> {
        let mut server_ids = IndexSet::new();
        for access_group_id in access_group_ids {
            match get_access_object(self.teams, access_group_id, requires_fresh_policy).await {
                Ok(group) => server_ids.extend(group.access_mcp_server_ids),
                Err(error) if requires_fresh_policy => {
                    return Err(GrantReadError::AccessGroup(error));
                }
                Err(error) => debug!(
                    "Could not fetch access group {access_group_id} for resource field access_mcp_server_ids: {error}"
                ),
            }
        }
        Ok(server_ids.into_iter().collect())
    }

    async fn servers_from_access_groups(
        &self,
        access_groups: &[String],
        requires_fresh_policy: bool,
    ) -> Result<Vec<String>, GrantReadError> {
        servers_from_access_groups(
            self.servers,
            self.server_rows.as_ref(),
            access_groups,
            requires_fresh_policy,
        )
        .await
        .map_err(|error| GrantReadError::AccessGroupServers(Box::new(error)))
    }
}
