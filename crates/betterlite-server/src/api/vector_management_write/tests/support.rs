#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::io;
use std::str;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
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
use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use chrono::Utc;
use tower::ServiceExt as _;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::super::cache::{CacheError, VectorStoreCache};
use super::super::routes;
use super::super::rows::{
    FieldChange, NewVectorStoreRecord, VectorStoreChanges, VectorStoreWrites,
};
use super::super::state::VectorManagementWriteState;
use crate::api::vector_management_boundaries::rows::VectorStoreRows;
use crate::api::vector_management_read::state::VectorManagementReadState;
use crate::auth::auth_identity::AuthContext;
use crate::management_admins::scope::AdminCaller;
use crate::management_rbac::guard::AdminPrivileges;

pub(super) const SIMULATED_FAILURE: &str = "simulated database failure";
pub(super) const SIMULATED_CACHE_FAILURE: &str = "simulated cache failure";
pub(super) const CREATED_AT: &str = "2024-01-02T03:04:05.000";
pub(super) const UPDATED_AT: &str = "2024-02-03T04:05:06.000";

type Found = Result<Option<LiteLlmManagedVectorStore>, TableError>;
type Stored = Result<LiteLlmManagedVectorStore, TableError>;
type Kept = Result<(), CacheError>;
type Rows<'guard> = MutexGuard<'guard, Vec<LiteLlmManagedVectorStore>>;

// a record built from the wire shape the DB hands over
pub(super) fn parsed(source: &str) -> LiteLlmManagedVectorStore {
    let tree: Json = serde_json::from_str(source).expect("the record is JSON");
    serde_json::to_value(tree)
        .and_then(serde_json::from_value)
        .expect("the record fits")
}

// a database row the way Prisma stores the two JSONB columns: JSON text, not objects
pub(super) fn db_store(id: &str) -> LiteLlmManagedVectorStore {
    parsed(&format!(
        r#"{{"vector_store_id":"{id}","custom_llm_provider":"openai","vector_store_name":"db-{id}","vector_store_metadata":"{{\"env\":\"prod\"}}","created_at":"{CREATED_AT}","updated_at":"{UPDATED_AT}","litellm_credential_name":null,"litellm_params":"{{\"api_key\":\"db-secret-{id}\",\"region\":\"us\"}}","team_id":null,"user_id":"u1"}}"#
    ))
}

pub(super) fn team_store(id: &str, team_id: &str) -> LiteLlmManagedVectorStore {
    let mut store = db_store(id);
    store.team_id = Some(team_id.to_owned());
    store
}

pub(super) fn config_store(id: &str) -> LiteLlmManagedVectorStore {
    parsed(&format!(
        r#"{{"vector_store_id":"{id}","custom_llm_provider":"openai","vector_store_name":"config-{id}","litellm_params":{{"api_key":"config-secret-{id}"}},"is_config":true}}"#
    ))
}

pub(super) fn memory_store(id: &str) -> LiteLlmManagedVectorStore {
    parsed(&format!(
        r#"{{"vector_store_id":"{id}","custom_llm_provider":"openai","litellm_params":{{"api_key":"memory-secret-{id}"}}}}"#
    ))
}

// every database and cache call in the order the route made it, so a test sees DB before cache
#[derive(Clone, Default)]
pub(super) struct Events {
    inner: Arc<Mutex<Vec<String>>>,
}

impl Events {
    pub(super) fn note(&self, entry: String) {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }

    pub(super) fn entries(&self) -> Vec<String> {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Find,
    Create,
    Update,
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CacheFailure {
    Add,
    Update,
    Remove,
}

struct DbInner {
    rows: Mutex<Vec<LiteLlmManagedVectorStore>>,
    creates: Mutex<Vec<NewVectorStoreRecord>>,
    updates: Mutex<Vec<(String, VectorStoreChanges)>>,
    failure: Mutex<Option<Failure>>,
    missing_update: Mutex<bool>,
}

#[derive(Clone)]
pub(super) struct FakeDatabase {
    inner: Arc<DbInner>,
    events: Events,
}

impl FakeDatabase {
    pub(super) fn new(rows: Vec<LiteLlmManagedVectorStore>) -> Self {
        let inner = DbInner {
            rows: Mutex::new(rows),
            creates: Mutex::new(Vec::new()),
            updates: Mutex::new(Vec::new()),
            failure: Mutex::new(None),
            missing_update: Mutex::new(false),
        };
        Self {
            inner: Arc::new(inner),
            events: Events::default(),
        }
    }

    pub(super) fn with_events(self, events: Events) -> Self {
        Self { events, ..self }
    }

    pub(super) fn failing(self, failure: Failure) -> Self {
        arm(&self.inner.failure, failure);
        self
    }

    // a row that vanished between the access fetch and the update: the repository returns None
    pub(super) fn missing_update(self) -> Self {
        *self
            .inner
            .missing_update
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = true;
        self
    }

    fn skips_update(&self) -> bool {
        *self
            .inner
            .missing_update
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn creates(&self) -> Vec<NewVectorStoreRecord> {
        self.inner
            .creates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn updates(&self) -> Vec<(String, VectorStoreChanges)> {
        self.inner
            .updates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn rows(&self) -> Vec<LiteLlmManagedVectorStore> {
        self.rows_lock().clone()
    }

    fn rows_lock(&self) -> Rows<'_> {
        self.inner
            .rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    // a database call's event and failure gate, then the rows it reads or changes
    fn begin(&self, entry: &str, failure: Failure) -> Result<Rows<'_>, TableError> {
        self.events.note(entry.to_owned());
        if armed(&self.inner.failure, failure) {
            return Err(simulated());
        }
        Ok(self.rows_lock())
    }

    fn find(&self, vector_store_id: &str) -> Found {
        let rows = self.begin("db:find", Failure::Find)?;
        Ok(rows
            .iter()
            .find(|row| row.vector_store_id.as_deref() == Some(vector_store_id))
            .cloned())
    }

    fn insert(&self, record: NewVectorStoreRecord) -> Stored {
        let store = record_to_store(&record);
        let mut rows = self.begin("db:create", Failure::Create)?;
        self.inner
            .creates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
        rows.push(store.clone());
        Ok(store)
    }

    fn replace(&self, vector_store_id: &str, changes: VectorStoreChanges) -> Found {
        let mut rows = self.begin("db:update", Failure::Update)?;
        self.inner
            .updates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((vector_store_id.to_owned(), changes.clone()));
        if self.skips_update() {
            return Ok(None);
        }
        let updated = rows
            .iter_mut()
            .find(|row| row.vector_store_id.as_deref() == Some(vector_store_id))
            .map(|row| {
                apply_changes(row, &changes);
                row.clone()
            });
        Ok(updated)
    }

    fn take(&self, vector_store_id: &str) -> Found {
        let mut rows = self.begin("db:delete", Failure::Delete)?;
        let Some(position) = rows
            .iter()
            .position(|row| row.vector_store_id.as_deref() == Some(vector_store_id))
        else {
            return Ok(None);
        };
        Ok(Some(rows.remove(position)))
    }
}

// a fake's failure cell: set by the test builder, read by the call the test blocks
fn arm<T>(cell: &Mutex<Option<T>>, failure: T) {
    *cell.lock().unwrap_or_else(PoisonError::into_inner) = Some(failure);
}

fn armed<T>(cell: &Mutex<Option<T>>, which: T) -> bool
where
    T: PartialEq,
{
    *cell.lock().unwrap_or_else(PoisonError::into_inner) == Some(which)
}

impl VectorStoreRows for FakeDatabase {
    fn find_unique(&self, vector_store_id: &str) -> impl Future<Output = Found> + Send {
        ready(self.find(vector_store_id))
    }
}

impl VectorStoreWrites for FakeDatabase {
    fn create(&self, record: NewVectorStoreRecord) -> impl Future<Output = Stored> + Send {
        ready(self.insert(record))
    }

    fn update(
        &self,
        vector_store_id: &str,
        changes: VectorStoreChanges,
    ) -> impl Future<Output = Found> + Send {
        ready(self.replace(vector_store_id, changes))
    }

    fn delete(&self, vector_store_id: &str) -> impl Future<Output = Found> + Send {
        ready(self.take(vector_store_id))
    }
}

fn simulated() -> TableError {
    TableError::Publish {
        message: String::from(SIMULATED_FAILURE),
    }
}

fn record_to_store(record: &NewVectorStoreRecord) -> LiteLlmManagedVectorStore {
    LiteLlmManagedVectorStore {
        vector_store_id: Some(record.vector_store_id.clone()),
        custom_llm_provider: Some(record.custom_llm_provider.clone()),
        vector_store_name: record.vector_store_name.clone(),
        vector_store_description: record.vector_store_description.clone(),
        vector_store_metadata: record
            .vector_store_metadata
            .as_deref()
            .map(|text| serde_json::to_value(Json::Text(text.to_owned())).expect("text is JSON")),
        created_at: Some(String::from(CREATED_AT)),
        updated_at: Some(String::from(UPDATED_AT)),
        litellm_credential_name: record.litellm_credential_name.clone(),
        litellm_params: Some(
            serde_json::to_value(Json::Text(record.litellm_params.clone())).expect("text is JSON"),
        ),
        team_id: record.team_id.clone(),
        user_id: record.user_id.clone(),
        is_config: None,
        extra: BTreeMap::new(),
    }
}

fn apply_changes(row: &mut LiteLlmManagedVectorStore, changes: &VectorStoreChanges) {
    apply(
        &mut row.custom_llm_provider,
        &changes.custom_llm_provider,
        |text: &str| text.to_owned(),
    );
    apply(
        &mut row.vector_store_name,
        &changes.vector_store_name,
        |text: &str| text.to_owned(),
    );
    apply(
        &mut row.vector_store_description,
        &changes.vector_store_description,
        |text: &str| text.to_owned(),
    );
    apply(
        &mut row.vector_store_metadata,
        &changes.vector_store_metadata,
        |text: &str| serde_json::to_value(Json::Text(text.to_owned())).expect("text is JSON"),
    );
    row.updated_at = Some(String::from(UPDATED_AT));
}

fn apply<T, F>(field: &mut Option<T>, change: &FieldChange<String>, convert: F)
where
    F: Fn(&str) -> T,
{
    match change {
        FieldChange::Omitted => {}
        FieldChange::Null => *field = None,
        FieldChange::Value(text) => *field = Some(convert(text)),
    }
}

#[derive(Default)]
struct CacheInner {
    stores: Mutex<Vec<LiteLlmManagedVectorStore>>,
    failure: Mutex<Option<CacheFailure>>,
}

#[derive(Clone, Default)]
pub(super) struct FakeCache {
    inner: Arc<CacheInner>,
    events: Events,
}

impl FakeCache {
    pub(super) fn new(stores: Vec<LiteLlmManagedVectorStore>) -> Self {
        let cache = Self::default();
        *cache.stores_lock() = stores;
        cache
    }

    pub(super) fn with_events(self, events: Events) -> Self {
        Self { events, ..self }
    }

    pub(super) fn failing(self, failure: CacheFailure) -> Self {
        arm(&self.inner.failure, failure);
        self
    }

    pub(super) fn stores(&self) -> Vec<LiteLlmManagedVectorStore> {
        self.stores_lock().clone()
    }

    fn stores_lock(&self) -> Rows<'_> {
        self.inner
            .stores
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    // a cache call's event and failure gate, then the stores it reads or changes
    fn gate(&self, entry: &str, failure: CacheFailure) -> Result<Rows<'_>, CacheError> {
        self.events.note(entry.to_owned());
        if armed(&self.inner.failure, failure) {
            return Err(simulated_cache());
        }
        Ok(self.stores_lock())
    }

    fn lookup(&self, vector_store_id: &str) -> Option<LiteLlmManagedVectorStore> {
        self.events.note(String::from("cache:find"));
        self.stores_lock()
            .iter()
            .find(|store| store.vector_store_id.as_deref() == Some(vector_store_id))
            .cloned()
    }

    fn insert(&self, store: LiteLlmManagedVectorStore) -> Kept {
        let mut stores = self.gate("cache:add", CacheFailure::Add)?;
        let known = store.vector_store_id.clone();
        if !stores.iter().any(|held| held.vector_store_id == known) {
            stores.push(store);
        }
        Ok(())
    }

    fn replace(&self, vector_store_id: &str, store: LiteLlmManagedVectorStore) -> Kept {
        let mut stores = self.gate("cache:update", CacheFailure::Update)?;
        let position = stores
            .iter()
            .position(|held| held.vector_store_id.as_deref() == Some(vector_store_id));
        match position {
            Some(index) => stores[index] = store,
            None => stores.push(store),
        }
        Ok(())
    }

    fn discard(&self, vector_store_id: &str) -> Kept {
        let mut stores = self.gate("cache:remove", CacheFailure::Remove)?;
        stores.retain(|held| held.vector_store_id.as_deref() != Some(vector_store_id));
        Ok(())
    }
}

impl VectorStoreCache for FakeCache {
    fn find(&self, vector_store_id: &str) -> Option<LiteLlmManagedVectorStore> {
        self.lookup(vector_store_id)
    }

    fn add(&self, store: LiteLlmManagedVectorStore) -> Kept {
        self.insert(store)
    }

    fn update(&self, vector_store_id: &str, store: LiteLlmManagedVectorStore) -> Kept {
        self.replace(vector_store_id, store)
    }

    fn remove(&self, vector_store_id: &str) -> Kept {
        self.discard(vector_store_id)
    }
}

fn simulated_cache() -> CacheError {
    CacheError {
        message: String::from(SIMULATED_CACHE_FAILURE),
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

pub(super) fn flags_open() -> FeatureSettings {
    FeatureSettings::default()
}

pub(super) fn flags_blocked() -> FeatureSettings {
    let mut settings = FeatureSettings::default();
    settings.vector_stores.disabled = true;
    settings
}

pub(super) fn flags_blocked_team_admins() -> FeatureSettings {
    let mut settings = flags_blocked();
    settings.vector_stores.team_admins_allowed = true;
    settings
}

#[derive(Default)]
pub(super) struct Setup {
    pub(super) database: Option<FakeDatabase>,
    pub(super) cache: Option<FakeCache>,
    pub(super) registry: Option<Vec<LiteLlmManagedVectorStore>>,
    pub(super) world: World,
    pub(super) privileges: bool,
    pub(super) settings: FeatureSettings,
    pub(super) events: Events,
}

pub(super) fn read_state(setup: Setup) -> Arc<VectorManagementReadState<FakeDatabase>> {
    let events = setup.events.clone();
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
        database: setup.database.map(|database| database.with_events(events)),
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

pub(super) fn build(setup: Setup) -> Arc<VectorManagementWriteState<FakeDatabase>> {
    let cache = setup.cache.clone();
    let events = setup.events.clone();
    let cache: Option<Arc<dyn VectorStoreCache>> =
        cache.map(|store| -> Arc<dyn VectorStoreCache> { Arc::new(store.with_events(events)) });
    Arc::new(VectorManagementWriteState {
        reads: read_state(setup),
        cache,
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

// one POST through the module's routes: the caller rides in the extensions the auth stage would fill
pub(super) async fn call<Database>(
    state: &Arc<VectorManagementWriteState<Database>>,
    caller: &AuthContext,
    uri: &str,
    payload: &str,
) -> (StatusCode, HeaderMap, Json)
where
    Database: VectorStoreWrites + Send + Sync + 'static,
{
    let request = Request::builder()
        .method("POST")
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
