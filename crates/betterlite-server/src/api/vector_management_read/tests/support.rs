#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::io;
use std::str;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_auth::auth_prefetch::permission::lookup::PermissionError;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_team_lookup::lookup::TeamRequest;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::identity_grants::loaders::TeamLoader;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::ui_session_identity::UiSessionTeams;
use betterlite_config::sections::management_rbac::Settings as FeatureSettings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::vector_store_access::auth::ObjectPermissionLookup;
use betterlite_core::api::vector_store_registry::repository::VectorStoreDatabase;
use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::pydantic::LiteLlmManagedVectorStoreIndex;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use chrono::Utc;
use tower::ServiceExt as _;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::super::routes;
use super::super::state::VectorManagementReadState;
use crate::api::vector_management_boundaries::rows::VectorStoreRows;
use crate::auth::auth_identity::AuthContext;
use crate::management_admins::scope::AdminCaller;
use crate::management_rbac::guard::AdminPrivileges;

// a record built from the wire shape the DB and the config loader hand over
pub(super) fn parsed(source: &str) -> LiteLlmManagedVectorStore {
    let tree: Json = serde_json::from_str(source).expect("the record is JSON");
    serde_json::to_value(tree)
        .and_then(serde_json::from_value)
        .expect("the record fits")
}

pub(super) fn db_store(id: &str) -> LiteLlmManagedVectorStore {
    parsed(&format!(
        r#"{{"vector_store_id":"{id}","custom_llm_provider":"openai","vector_store_name":"db-{id}","vector_store_metadata":{{"env":"prod"}},"created_at":"2024-01-02T03:04:05Z","litellm_credential_name":null,"litellm_params":{{"api_key":"db-secret-{id}","region":"us"}},"team_id":null,"user_id":"u1"}}"#
    ))
}

pub(super) fn config_store(id: &str) -> LiteLlmManagedVectorStore {
    parsed(&format!(
        r#"{{"vector_store_id":"{id}","custom_llm_provider":"openai","vector_store_name":"config-{id}","vector_store_metadata":{{"env":"prod"}},"litellm_credential_name":null,"litellm_params":{{"api_key":"config-secret-{id}"}},"is_config":true}}"#
    ))
}

pub(super) fn memory_store(id: &str) -> LiteLlmManagedVectorStore {
    parsed(&format!(
        r#"{{"vector_store_id":"{id}","custom_llm_provider":"openai","litellm_params":{{"api_key":"memory-secret-{id}"}}}}"#
    ))
}

pub(super) fn team_store(id: &str, team_id: &str) -> LiteLlmManagedVectorStore {
    let mut store = db_store(id);
    store.team_id = Some(team_id.to_owned());
    store
}

#[derive(Default)]
struct FakeInner {
    rows: Vec<LiteLlmManagedVectorStore>,
    store_calls: AtomicUsize,
    unique_calls: Mutex<Vec<String>>,
}

// the database the routes read, shared so a test sees the queries the handler made
#[derive(Clone, Default)]
pub(super) struct FakeDatabase {
    inner: Arc<FakeInner>,
}

impl FakeDatabase {
    pub(super) fn new(rows: Vec<LiteLlmManagedVectorStore>) -> Self {
        Self {
            inner: Arc::new(FakeInner {
                rows,
                store_calls: AtomicUsize::new(0),
                unique_calls: Mutex::new(Vec::new()),
            }),
        }
    }

    pub(super) fn asked(&self) -> Vec<String> {
        self.inner
            .unique_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn loads(&self) -> usize {
        self.inner.store_calls.load(Ordering::SeqCst)
    }
}

impl VectorStoreDatabase for FakeDatabase {
    async fn stores(&self) -> Result<Vec<LiteLlmManagedVectorStore>, TableError> {
        self.inner.store_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.inner.rows.clone())
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
        self.inner
            .unique_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(vector_store_id.to_owned());
        Ok(self
            .inner
            .rows
            .iter()
            .find(|row| row.vector_store_id.as_deref() == Some(vector_store_id))
            .cloned())
    }
}

// the grant lookups: no object permissions, and a team loader that never finds the team
#[derive(Default)]
pub(super) struct World {
    pub(super) user_teams: Vec<String>,
    pub(super) database: bool,
}

#[async_trait]
impl ObjectPermissionLookup for World {
    fn has_database(&self) -> bool {
        self.database
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
        self.database
    }

    async fn user_teams(
        &self,
        _user_id: &str,
        _fresh: bool,
    ) -> Result<Option<Vec<String>>, UserLookupError> {
        Ok(Some(self.user_teams.clone()))
    }
}

pub(super) struct FakePrivileges {
    pub(super) granted: bool,
}

#[async_trait]
impl AdminPrivileges for FakePrivileges {
    type Failure = Infallible;

    async fn has_admin_privileges(&self, _caller: &AdminCaller<'_>) -> Result<bool, Infallible> {
        Ok(self.granted)
    }
}

// rbac_utils.py reads both switches as absent, so nothing is disabled
pub(super) fn flags_open() -> FeatureSettings {
    FeatureSettings::default()
}

pub(super) fn flags_blocked() -> FeatureSettings {
    let mut settings = FeatureSettings::default();
    settings.vector_stores.disabled = true;
    settings
}

// disable_vector_stores_for_internal_users with allow_vector_stores_for_team_admins
pub(super) fn flags_blocked_team_admins() -> FeatureSettings {
    let mut settings = flags_blocked();
    settings.vector_stores.team_admins_allowed = true;
    settings
}

#[derive(Default)]
pub(super) struct Setup {
    pub(super) database: Option<FakeDatabase>,
    pub(super) registry: Option<Vec<LiteLlmManagedVectorStore>>,
    pub(super) world: World,
    pub(super) privileges: bool,
    pub(super) settings: FeatureSettings,
}

pub(super) fn build(setup: Setup) -> Arc<VectorManagementReadState<FakeDatabase>> {
    let world = Arc::new(setup.world);
    let registry = setup.registry.map(|stores| {
        Arc::new(Mutex::new(VectorStoreRegistry {
            stores,
            clock: Arc::new(Utc::now),
            credentials: Arc::new(CredentialSnapshot::default()),
        }))
    });
    let permissions = Arc::clone(&world);
    let teams = Arc::clone(&world);
    Arc::new(VectorManagementReadState {
        database: setup.database,
        registry,
        permissions,
        teams,
        session_teams: world,
        privileges: Arc::new(FakePrivileges {
            granted: setup.privileges,
        }),
        settings: setup.settings,
        limits: ApiLimits::default(),
    })
}

fn principal(roles: &[Role], key: Option<UserAPIKeyAuth>) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: "caller".to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: roles.to_vec(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey(key),
        },
    }
}

pub(super) fn admin() -> AuthContext {
    principal(&[Role::PlatformAdmin], None)
}

pub(super) fn viewer() -> AuthContext {
    principal(&[Role::PlatformViewer], None)
}

// a keyed caller: internal_user, optionally acting under a team
pub(super) fn internal(user_id: &str, team_id: Option<&str>) -> AuthContext {
    let mut fields = BTreeMap::new();
    fields.insert("user_id".to_owned(), JsonData::String(user_id.to_owned()));
    fields.insert(
        "user_role".to_owned(),
        JsonData::String("internal_user".to_owned()),
    );
    if let Some(team) = team_id {
        fields.insert("team_id".to_owned(), JsonData::String(team.to_owned()));
    }
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the key is valid");
    principal(&[], Some(key))
}

// a dashboard-minted session token: the litellm-dashboard team plus a signed-in user
pub(super) fn ui_session(user_id: &str) -> AuthContext {
    let mut fields = BTreeMap::new();
    fields.insert("user_id".to_owned(), JsonData::String(user_id.to_owned()));
    fields.insert(
        "user_role".to_owned(),
        JsonData::String("internal_user".to_owned()),
    );
    fields.insert(
        "team_id".to_owned(),
        JsonData::String("litellm-dashboard".to_owned()),
    );
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the key is valid");
    principal(&[], Some(key))
}

pub(super) fn registry_of(
    state: &Arc<VectorManagementReadState<FakeDatabase>>,
) -> MutexGuard<'_, VectorStoreRegistry> {
    let registry = state
        .registry
        .as_ref()
        .expect("the state carries a registry");
    registry.lock().unwrap_or_else(PoisonError::into_inner)
}

// one request through the module's routes: the caller rides in the extensions the auth stage would fill
pub(super) async fn call(
    state: &Arc<VectorManagementReadState<FakeDatabase>>,
    caller: &AuthContext,
    method: Method,
    uri: &str,
    payload: &str,
) -> (StatusCode, HeaderMap, Json) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .extension(caller.clone())
        .body(Body::from(payload.to_owned()))
        .expect("the request builds");
    let response = routes(Arc::clone(state))
        .oneshot(request)
        .await
        .expect("the router answers");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body reads");
    let body: Json = serde_json::from_slice(&bytes).expect("the body is JSON");
    (status, headers, body)
}

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

// the module's own log lines, captured on the calling thread at the given level
pub(super) fn capture_logs(level: tracing::Level) -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(level)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
