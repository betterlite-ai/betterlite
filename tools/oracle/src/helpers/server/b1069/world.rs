use std::collections::BTreeMap;
use std::convert::Infallible;

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_auth::auth_prefetch::permission::lookup::PermissionError;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_team_lookup::lookup::TeamRequest;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::identity_grants::loaders::TeamLoader;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::ui_session_identity::UiSessionTeams;
use betterlite_core::api::vector_store_access::auth::ObjectPermissionLookup;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_server::auth::auth_identity::AuthContext;
use betterlite_server::management_admins::scope::AdminCaller;
use betterlite_server::management_rbac::guard::AdminPrivileges;

use super::input::Caller;

// no grants and no team rows, like a proxy whose keys carry no object permissions
pub(super) struct World;

#[async_trait]
impl ObjectPermissionLookup for World {
    fn has_database(&self) -> bool {
        false
    }

    async fn load_permission(
        &self,
        _object_permission_id: &str,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError> {
        Ok(None)
    }
}

#[async_trait]
impl TeamLoader for World {
    async fn load_team(
        &self,
        request: TeamRequest<'_>,
    ) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError> {
        Err(TeamLookupError::TeamNotFound {
            team_id: request.team_id.to_owned(),
        })
    }
}

#[async_trait]
impl UiSessionTeams for World {
    fn has_database(&self) -> bool {
        false
    }

    async fn user_teams(
        &self,
        _user_id: &str,
        _fresh: bool,
    ) -> Result<Option<Vec<String>>, UserLookupError> {
        Ok(Some(Vec::new()))
    }
}

pub(super) struct NoPrivileges;

#[async_trait]
impl AdminPrivileges for NoPrivileges {
    type Failure = Infallible;

    async fn has_admin_privileges(&self, _caller: &AdminCaller<'_>) -> Result<bool, Infallible> {
        Ok(false)
    }
}

pub(super) fn auth_context(caller: &Caller) -> Result<AuthContext, Box<HelperError>> {
    let mut fields = BTreeMap::new();
    fields.insert(
        "user_id".to_owned(),
        JsonData::String(caller.user_id.clone()),
    );
    fields.insert(
        "user_role".to_owned(),
        JsonData::String(caller.user_role.clone()),
    );
    if let Some(team_id) = &caller.team_id {
        fields.insert("team_id".to_owned(), JsonData::String(team_id.clone()));
    }
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(fields))
        .map_err(|error| failure(error.to_string()))?;
    Ok(AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: "oracle".to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: Vec::new(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey(Some(key)),
        },
    })
}

pub(super) fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "Internal".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
