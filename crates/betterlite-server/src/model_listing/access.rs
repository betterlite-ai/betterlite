use std::collections::BTreeSet;

use async_trait::async_trait;
use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_auth::auth_model_lists::providers::ProviderModels;
use betterlite_auth::auth_model_lists::wildcard::{DeploymentLookup, ExpansionContext};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::budget_permission_rows::JsonData;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::model_listing::ModelListingError;

// get_model_listing_info's answer: the cost map keys behind one listed name and the limits it declares.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeploymentListingInfo {
    pub cost_map_keys: Vec<String>,
    pub max_input_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
}

// The router facts a listing reads: its names, its access groups, one name's deployments and its info.
pub trait ListingRouter: DeploymentLookup {
    fn model_names(&self) -> Vec<String>;
    fn model_access_groups(&self) -> IndexMap<String, Vec<String>>;
    fn model_group_alias(&self, model: &str) -> Option<String>;
    fn listing_info(&self, model: &str) -> Option<DeploymentListingInfo>;
    fn configured_mode(&self, model: &str) -> Option<String>;
}

// The team fields a listing reads: its id, its granted models and its access groups.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TeamListing {
    pub team_id: String,
    pub models: Vec<String>,
    pub access_group_ids: Vec<String>,
}

// A requested team the database does not have; LiteLLM's get_team_object raises 404 whatever failed.
#[derive(Debug, thiserror::Error)]
#[error("the team {team_id:?} does not exist")]
pub struct TeamMissing {
    pub team_id: String,
}

// A key whose caller is not on the team it names; LiteLLM's validate_membership raises 403 for it.
#[derive(Debug, thiserror::Error)]
#[error("the caller is not a member of team {team_id:?}")]
pub struct NotAMember {
    pub team_id: String,
}

// The team, membership, access group and agent ceiling reads a listing makes, injected so a test fakes them.
#[async_trait]
pub trait AccessLookups: Send + Sync {
    // Ok(None) when no database, logging or key cache is configured, so no team is looked up at all
    async fn team(&self, team_id: &str) -> Result<Option<TeamListing>, TeamMissing>;
    async fn validate_membership(
        &self,
        key: &UserAPIKeyAuth,
        team: &TeamListing,
    ) -> Result<(), NotAMember>;
    async fn team_access_group_models(&self, team: Option<&TeamListing>) -> Vec<String>;
    async fn key_access_group_models(
        &self,
        key: &UserAPIKeyAuth,
        team: Option<&TeamListing>,
    ) -> Vec<String>;
    async fn agent_ceiling(&self, agent_id: &str) -> Option<BTreeSet<String>>;
}

// What a listing reads from outside: the router, the access lookups and the catalogue a wildcard expands through.
pub struct ListingSources<'sources> {
    pub router: Option<&'sources dyn ListingRouter>,
    pub access: &'sources dyn AccessLookups,
    pub providers: &'sources dyn ProviderModels,
    pub credentials: &'sources CredentialSnapshot,
}

impl ListingSources<'_> {
    // the expansion context every wildcard in this listing runs in
    #[must_use]
    pub fn expansion_context(&self) -> ExpansionContext<'_> {
        ExpansionContext {
            providers: self.providers,
            credentials: self.credentials,
            // the field's type upcasts the listing router to its deployment lookup supertrait
            deployments: self.router.map(|router: &dyn ListingRouter| router),
        }
    }
}

// The UserAPIKeyAuth fields get_available_models_for_user reads, taken from the key's serde form.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct KeyModelFields {
    pub models: Vec<JsonData>,
    pub team_models: Vec<JsonData>,
    pub team_id: Option<String>,
    pub agent_id: Option<String>,
}

impl KeyModelFields {
    pub fn from_key(key: &UserAPIKeyAuth) -> Result<Self, ModelListingError> {
        serde_json::from_value(serde_json::to_value(key).map_err(ModelListingError::KeyFields)?)
            .map_err(ModelListingError::KeyFields)
    }
}

// The router's names and access groups, read once so a listing asks the router for each answer once.
#[derive(Debug, Default)]
pub struct RouterCatalog {
    pub model_names: Vec<String>,
    pub access_groups: IndexMap<String, Vec<String>>,
}

impl RouterCatalog {
    #[must_use]
    pub fn read(router: Option<&dyn ListingRouter>) -> Self {
        let Some(router) = router else {
            return Self::default();
        };
        Self {
            model_names: router.model_names(),
            access_groups: router.model_access_groups(),
        }
    }
}

// List[str] in LiteLLM: a stored grant entry that is not text grants nothing.
#[must_use]
pub fn model_names(values: &[JsonData]) -> Vec<String> {
    values
        .iter()
        .filter_map(|value| match value {
            JsonData::String(name) => Some(name.clone()),
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::Array(_)
            | JsonData::Object(_) => None,
        })
        .collect()
}
