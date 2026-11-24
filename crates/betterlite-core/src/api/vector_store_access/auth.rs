use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};

use async_trait::async_trait;
use betterlite_auth::auth_prefetch::permission::lookup::PermissionError;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
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

// the credential fields grants read, in the shape LiteLLM's model_copy leaves them
#[derive(Clone, Eq, PartialEq)]
pub struct AuthContext {
    pub team_id: Option<String>,
    pub object_permission: Option<LiteLLMObjectPermissionTable>,
    pub object_permission_id: Option<String>,
    pub team_object_permission: Option<LiteLLMObjectPermissionTable>,
    pub team_object_permission_id: Option<String>,
    // the whole credential as fields, with the five above replaced, so a caller rebuilds or narrows it
    pub fields: BTreeMap<String, JsonData>,
}

impl Debug for AuthContext {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        // fields carries the credential's api key, so Debug prints ids and a count, never a field value
        formatter
            .debug_struct("AuthContext")
            .field("team_id", &self.team_id)
            .field("object_permission", &self.object_permission)
            .field("object_permission_id", &self.object_permission_id)
            .field("team_object_permission", &self.team_object_permission)
            .field("team_object_permission_id", &self.team_object_permission_id)
            .field("field_count", &self.fields.len())
            .finish()
    }
}

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
    // the credential's policy fields plus every other field it serializes
    pub fn from_auth(auth: &UserAPIKeyAuth) -> Result<Self, VectorStoreAccessError> {
        let fields: BTreeMap<String, JsonData> = serde_json::to_value(auth)
            .and_then(serde_json::from_value)
            .map_err(VectorStoreAccessError::Fields)?;
        let parsed: AuthFields = serde_json::to_value(auth)
            .and_then(serde_json::from_value)
            .map_err(VectorStoreAccessError::Fields)?;
        Ok(Self {
            team_id: parsed.team_id,
            object_permission: parsed.object_permission,
            object_permission_id: parsed.object_permission_id,
            team_object_permission: parsed.team_object_permission,
            team_object_permission_id: parsed.team_object_permission_id,
            fields,
        })
    }

    // _team_auth_context's update: the row's team fields replace the credential's, every other field stays
    pub(super) fn with_team(
        mut self,
        team_id: &str,
        team: &LiteLLMTeamTableCachedObj,
    ) -> Result<Self, VectorStoreAccessError> {
        let permission = team.team.fields.object_permission.clone();
        let permission_id = team.team.fields.object_permission_id.clone();
        self.team_id = Some(team_id.to_owned());
        self.team_object_permission = permission.clone();
        self.team_object_permission_id = permission_id.clone();
        self.fields
            .insert("team_id".to_owned(), JsonData::String(team_id.to_owned()));
        self.fields.insert(
            "team_object_permission_id".to_owned(),
            optional_text(permission_id),
        );
        let permission = serde_json::to_value(permission)
            .and_then(serde_json::from_value)
            .map_err(VectorStoreAccessError::Fields)?;
        self.fields
            .insert("team_object_permission".to_owned(), permission);
        Ok(self)
    }

    // _vector_store_auth_contexts' session context: the synthetic team's fields are cleared, key grants stay
    pub(super) fn without_team(&mut self) {
        self.team_id = None;
        self.team_object_permission = None;
        self.team_object_permission_id = None;
        self.fields.insert("team_id".to_owned(), JsonData::Null);
        self.fields
            .insert("team_object_permission".to_owned(), JsonData::Null);
        self.fields
            .insert("team_object_permission_id".to_owned(), JsonData::Null);
    }
}

fn optional_text(value: Option<String>) -> JsonData {
    match value {
        Some(text) => JsonData::String(text),
        None => JsonData::Null,
    }
}
