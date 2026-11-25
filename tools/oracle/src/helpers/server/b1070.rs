mod input;

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use self::input::{Call, Caller, Input, Setup};
use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::Request;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_auth::auth_prefetch::permission::lookup::PermissionError;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_team_lookup::lookup::TeamRequest;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::identity_grants::loaders::TeamLoader;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::ui_session_identity::UiSessionTeams;
use betterlite_config::sections::management_rbac::{FeatureSwitches, Settings};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::vector_store_access::auth::ObjectPermissionLookup;
use betterlite_core::api::vector_store_registry::repository::VectorStoreDatabase;
use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_openai::api::vector_store_records::records::pydantic::LiteLlmManagedVectorStoreIndex;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::api::vector_management_boundaries::rows::VectorStoreRows;
use betterlite_server::api::vector_management_read::state::VectorManagementReadState;
use betterlite_server::api::vector_management_read::{INFO_PATH, LIST_PATH, V1_LIST_PATH, routes};
use betterlite_server::auth::auth_identity::AuthContext;
use betterlite_server::management_admins::scope::AdminCaller;
use betterlite_server::management_rbac::guard::AdminPrivileges;
use chrono::Utc;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tower::ServiceExt as _;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        runtime.block_on(async {
            let mut answers: Vec<Value> = Vec::with_capacity(input.calls.len());
            for call in &input.calls {
                let (request, setup) = request_for(call)?;
                answers.push(answer(build_state(setup), request).await?);
            }
            Ok(json!(answers))
        })
    }
}

// the request a call names: the list's page query or the info body, both carrying the caller's context
fn request_for(call: &Call) -> Result<(Request<Body>, &Setup), Box<HelperError>> {
    let (setup, method, uri, body) = match call {
        Call::List(list) => (
            &list.setup,
            "GET",
            format!(
                "{}?page={}&page_size={}",
                list_path(list.alias),
                list.page,
                list.page_size
            ),
            Body::empty(),
        ),
        Call::Info(info) => (
            &info.setup,
            "POST",
            INFO_PATH.to_owned(),
            Body::from(json!({"vector_store_id": info.vector_store_id}).to_string()),
        ),
    };
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .extension(auth_context(&setup.caller)?)
        .body(body)
        .map_err(|error| failure(error.to_string()))?;
    Ok((request, setup))
}

const fn list_path(alias: bool) -> &'static str {
    if alias { V1_LIST_PATH } else { LIST_PATH }
}

// the HTTP answer both helpers compare: the status and the response body as JSON
async fn answer(
    state: Arc<VectorManagementReadState<FakeDatabase>>,
    request: Request<Body>,
) -> HelperResult {
    let response = routes(state)
        .oneshot(request)
        .await
        .map_err(|error| failure(error.to_string()))?;
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|error| failure(error.to_string()))?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|error| failure(error.to_string()))?;
    Ok(json!({"status": status, "body": body}))
}

fn build_state(setup: &Setup) -> Arc<VectorManagementReadState<FakeDatabase>> {
    let world = Arc::new(World);
    Arc::new(VectorManagementReadState {
        database: setup.database.clone().map(|rows| FakeDatabase { rows }),
        registry: setup.registry.clone().map(|stores| {
            Arc::new(Mutex::new(VectorStoreRegistry {
                stores,
                clock: Arc::new(Utc::now),
                credentials: Arc::new(CredentialSnapshot::default()),
            }))
        }),
        permissions: Arc::clone(&world),
        teams: Arc::clone(&world),
        session_teams: world,
        privileges: Arc::new(NoPrivileges),
        settings: Settings {
            agents: FeatureSwitches::default(),
            vector_stores: FeatureSwitches {
                disabled: setup.disabled,
                team_admins_allowed: false,
            },
        },
        limits: ApiLimits::default(),
    })
}

fn auth_context(caller: &Caller) -> Result<AuthContext, Box<HelperError>> {
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

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "Internal".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

#[derive(Clone)]
struct FakeDatabase {
    rows: Vec<LiteLlmManagedVectorStore>,
}

impl VectorStoreDatabase for FakeDatabase {
    async fn stores(&self) -> Result<Vec<LiteLlmManagedVectorStore>, TableError> {
        Ok(self.rows.clone())
    }

    async fn indexes(&self) -> Result<Vec<LiteLlmManagedVectorStoreIndex>, TableError> {
        Ok(Vec::new())
    }
}

impl VectorStoreRows for FakeDatabase {
    async fn find_unique(
        &self,
        vector_store_id: &str,
    ) -> Result<Option<LiteLlmManagedVectorStore>, TableError> {
        Ok(self
            .rows
            .iter()
            .find(|row| row.vector_store_id.as_deref() == Some(vector_store_id))
            .cloned())
    }
}

// no grants and no team rows, like a proxy whose keys carry no object permissions
struct World;

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

struct NoPrivileges;

#[async_trait]
impl AdminPrivileges for NoPrivileges {
    type Failure = Infallible;

    async fn has_admin_privileges(&self, _caller: &AdminCaller<'_>) -> Result<bool, Infallible> {
        Ok(false)
    }
}
