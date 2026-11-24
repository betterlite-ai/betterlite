use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_auth::auth_prefetch::permission::lookup::PermissionError;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_team_lookup::lookup::TeamRequest;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::identity_grants::loaders::TeamLoader;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::ui_session_identity::UiSessionTeams;
use betterlite_core::api::vector_store_access::VectorStoreAccess;
use betterlite_core::api::vector_store_access::auth::ObjectPermissionLookup;
use betterlite_core::api::vector_store_access::error::VectorStoreAccessError;
use betterlite_core::api::vector_store_access::store::{
    ManagedVectorStore, normalize_litellm_params,
};
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Runtime;

mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ParameterStringNormalization(NormalizedStoreInput),
    CanUserAccessVectorStore(AccessInput),
    FilterListableVectorStores(ListInput),
}

#[derive(Debug, Deserialize)]
pub(crate) struct NormalizedStoreInput {
    vector_store: ManagedVectorStore,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AccessInput {
    vector_store: ManagedVectorStore,
    #[serde(flatten)]
    world: WorldInput,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListInput {
    vector_stores: Vec<ManagedVectorStore>,
    #[serde(flatten)]
    world: WorldInput,
}

// the proxy globals a grant reads: the credential, whether a database exists, and the rows it loads
#[derive(Debug, Deserialize)]
struct WorldInput {
    auth: BTreeMap<String, JsonData>,
    #[serde(default = "default_database")]
    database: bool,
    #[serde(default)]
    object_permissions: BTreeMap<String, LiteLLMObjectPermissionTable>,
    #[serde(default)]
    teams: BTreeMap<String, LiteLLMTeamTableCachedObj>,
    #[serde(default)]
    user_teams: Vec<String>,
}

const fn default_database() -> bool {
    true
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input {
            Input::ParameterStringNormalization(store) => normalized(&store.vector_store),
            Input::CanUserAccessVectorStore(fields) => access(fields),
            Input::FilterListableVectorStores(fields) => listing(fields),
        }
    }
}

fn normalized(vector_store: &ManagedVectorStore) -> HelperResult {
    let store = normalize_litellm_params(vector_store);
    Ok(serde_json::to_value(store).or_value_error()?)
}

fn access(fields: &AccessInput) -> HelperResult {
    let scripted = Scripted {
        world: &fields.world,
    };
    let checker = policy(&scripted);
    let auth = fields.world.credential()?;
    let executor = runtime()?;
    let allowed = executor
        .block_on(checker.can_user_access_vector_store(&fields.vector_store, &auth))
        .map_err(|error| denied(&error))?;
    Ok(json!(allowed))
}

fn listing(fields: &ListInput) -> HelperResult {
    let scripted = Scripted {
        world: &fields.world,
    };
    let checker = policy(&scripted);
    let auth = fields.world.credential()?;
    let executor = runtime()?;
    let listable = executor
        .block_on(checker.filter_listable_vector_stores(&fields.vector_stores, &auth))
        .map_err(|error| denied(&error))?;
    Ok(json!(listable))
}

fn policy<'access, 'world>(
    scripted: &'access Scripted<'world>,
) -> VectorStoreAccess<'access, Scripted<'world>, Scripted<'world>, Scripted<'world>> {
    VectorStoreAccess {
        permissions: scripted,
        teams: scripted,
        session_teams: scripted,
    }
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    Ok(runtime)
}

fn denied(error: &VectorStoreAccessError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("HTTPException"),
        status: error.http(),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}

struct Scripted<'world> {
    world: &'world WorldInput,
}

impl WorldInput {
    fn credential(&self) -> Result<UserAPIKeyAuth, Box<HelperError>> {
        UserAPIKeyAuth::construct(AuthInput::Dict(self.auth.clone())).or_value_error()
    }
}

#[async_trait]
impl ObjectPermissionLookup for Scripted<'_> {
    fn has_database(&self) -> bool {
        self.world.database
    }

    async fn load_permission(
        &self,
        object_permission_id: &str,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError> {
        Ok(self
            .world
            .object_permissions
            .get(object_permission_id)
            .cloned())
    }
}

#[async_trait]
impl TeamLoader for Scripted<'_> {
    async fn load_team(
        &self,
        request: TeamRequest<'_>,
    ) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError> {
        match self.world.teams.get(request.team_id) {
            Some(team) => Ok(team.clone()),
            None => Err(TeamLookupError::TeamNotFound {
                team_id: request.team_id.to_owned(),
            }),
        }
    }
}

#[async_trait]
impl UiSessionTeams for Scripted<'_> {
    fn has_database(&self) -> bool {
        self.world.database
    }

    async fn user_teams(
        &self,
        _user_id: &str,
        _fresh: bool,
    ) -> Result<Option<Vec<String>>, UserLookupError> {
        Ok(Some(self.world.user_teams.clone()))
    }
}
