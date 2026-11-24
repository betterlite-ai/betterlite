use async_trait::async_trait;
use betterlite_auth::auth_prefetch::permission::lookup::PermissionError;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use serde::Deserialize;

use super::error::VectorStoreAccessError;

// B735's object permission read, as this policy needs it: whether a database exists and rows by id
#[async_trait]
pub trait ObjectPermissionLookup: Send + Sync {
    fn has_database(&self) -> bool;

    async fn load_permission(
        &self,
        object_permission_id: &str,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError>;
}

// the credential kept whole, so its api key and token stay SecretString, plus the five fields grants read
#[derive(Clone, Debug)]
pub struct AuthContext {
    pub credential: UserAPIKeyAuth,
    pub team_id: Option<String>,
    pub object_permission: Option<LiteLLMObjectPermissionTable>,
    pub object_permission_id: Option<String>,
    pub team_object_permission: Option<LiteLLMObjectPermissionTable>,
    pub team_object_permission_id: Option<String>,
}

// only the fields the policy decides on, projected out of the credential's own serialization
#[derive(Deserialize)]
struct AuthFields {
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default)]
    object_permission: Option<LiteLLMObjectPermissionTable>,
    #[serde(default)]
    object_permission_id: Option<String>,
    #[serde(default)]
    team_object_permission: Option<LiteLLMObjectPermissionTable>,
    #[serde(default)]
    team_object_permission_id: Option<String>,
}

impl AuthContext {
    pub fn from_auth(auth: &UserAPIKeyAuth) -> Result<Self, VectorStoreAccessError> {
        let parsed: AuthFields = serde_json::to_value(auth)
            .and_then(serde_json::from_value)
            .map_err(VectorStoreAccessError::Fields)?;
        Ok(Self {
            credential: auth.clone(),
            team_id: parsed.team_id,
            object_permission: parsed.object_permission,
            object_permission_id: parsed.object_permission_id,
            team_object_permission: parsed.team_object_permission,
            team_object_permission_id: parsed.team_object_permission_id,
        })
    }

    // _team_auth_context's update: the row's team fields replace the credential's, every other field stays
    pub(super) fn with_team(&mut self, team_id: &str, team: &LiteLLMTeamTableCachedObj) {
        self.team_id = Some(team_id.to_owned());
        self.team_object_permission
            .clone_from(&team.team.fields.object_permission);
        self.team_object_permission_id
            .clone_from(&team.team.fields.object_permission_id);
    }

    // _vector_store_auth_contexts' session context: the synthetic team's fields are cleared, key grants stay
    pub(super) fn without_team(&mut self) {
        self.team_id = None;
        self.team_object_permission = None;
        self.team_object_permission_id = None;
    }
}
