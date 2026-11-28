#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_auth::auth_identity::records::{Principal, SourceKey};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::mcp_management_mode::ManagementMode;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp::table_access::types::McpTableError;
use betterlite_db::mcp_user_env_vars::error::UserEnvVarsError;
use betterlite_db::mcp_user_env_vars::store::UserEnvVars;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_mcp::registry::resolution::{
    DbLookup, LookupFuture, ResolveError, ServerAccessCheck,
};
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry, SharedRegistry};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::sync::RwLock;
use tower::ServiceExt as _;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::auth::auth_identity::AuthContext;
use crate::mcp::management_server_access::AccessibleServerSources;
use crate::mcp_user_env_routes::UserEnvRoutesState;
use crate::mcp_user_env_routes::ports::{UserEnvVarsCache, UserEnvVarsStore};
use crate::mcp_user_env_routes::router::{SERVER_ENV_VARS_PATH, routes};

pub(crate) const SERVER_ONE: &str = r#"{"server_id":"srv-1","server_name":"Server One","alias":"one","transport":"http","env_vars":[{"name":"TOKEN","scope":"user","description":"Paste the token"}],"static_headers":{"Authorization":"Bearer ${TOKEN}"}}"#;
pub(crate) const SERVER_TWO: &str = r#"{"server_id":"srv-2","server_name":"Server Two","transport":"http","env_vars":[{"name":"OTHER","scope":"user"}],"static_headers":{"X-Other":"${OTHER}"}}"#;
pub(crate) const SERVER_BARE: &str =
    r#"{"server_id":"srv-3","server_name":"Bare Server","transport":"http"}"#;
pub(crate) const SERVER_ANON: &str = r#"{"server_id":"srv-1","transport":"http"}"#;
pub(crate) const REGISTRY_SERVER: &str =
    r#"{"server_id":"srv-9","name":"Registry Server","transport":"http"}"#;

pub(crate) const MEMBER: &str = r#"{"user_id":"user-1"}"#;
pub(crate) const OTHER_MEMBER: &str = r#"{"user_id":"user-2"}"#;
pub(crate) const ADMIN: &str = r#"{"user_id":"admin-1","user_role":"proxy_admin"}"#;
pub(crate) const NO_USER: &str = r#"{"user_role":"proxy_admin"}"#;

pub(crate) fn server_uri(server_id: &str) -> String {
    SERVER_ENV_VARS_PATH.replace("{server_id}", server_id)
}

fn key(fields: &str) -> UserAPIKeyAuth {
    let decoded: BTreeMap<String, JsonData> = serde_json::from_str(fields).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).unwrap()
}

pub(crate) fn context(fields: &str) -> AuthContext {
    let raw = r#"{"principal_type":"human","subject":"caller","roles":[],"auth_method":"api_key"}"#;
    let mut principal: Principal = serde_json::from_str(raw).unwrap();
    principal.source_key = SourceKey(Some(key(fields)));
    AuthContext { principal }
}

pub(crate) fn plain(values: &UserEnvVars) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

pub(crate) fn expected(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| (String::from(*name), String::from(*value)))
        .collect()
}

pub(crate) fn names(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| String::from(*name)).collect()
}

#[derive(Clone, Default)]
pub(crate) struct Journal(Arc<Mutex<Vec<String>>>);

impl Journal {
    pub(crate) fn record(&self, event: String) {
        self.0.lock().unwrap().push(event);
    }

    pub(crate) fn entries(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

pub(crate) struct FakeStore {
    journal: Journal,
    rows: Mutex<BTreeMap<(String, String), UserEnvVars>>,
    merges: Mutex<Vec<MergeCall>>,
    fail_get: AtomicBool,
    fail_bulk: AtomicBool,
    fail_merge: AtomicBool,
    fail_delete: AtomicBool,
}

#[derive(Clone)]
pub(crate) struct MergeCall {
    pub(crate) user_id: String,
    pub(crate) server_id: String,
    pub(crate) updates: UserEnvVars,
    pub(crate) allowed_names: BTreeSet<String>,
}

impl FakeStore {
    fn new(journal: Journal) -> Self {
        Self {
            journal,
            rows: Mutex::new(BTreeMap::new()),
            merges: Mutex::new(Vec::new()),
            fail_get: AtomicBool::new(false),
            fail_bulk: AtomicBool::new(false),
            fail_merge: AtomicBool::new(false),
            fail_delete: AtomicBool::new(false),
        }
    }

    pub(crate) fn seed(&self, user_id: &str, server_id: &str, pairs: &[(&str, &str)]) {
        let values = pairs
            .iter()
            .map(|(name, value)| (String::from(*name), SecretString::from(*value)))
            .collect();
        self.rows
            .lock()
            .unwrap()
            .insert((user_id.to_owned(), server_id.to_owned()), values);
    }

    pub(crate) fn row(&self, user_id: &str, server_id: &str) -> Option<UserEnvVars> {
        self.rows
            .lock()
            .unwrap()
            .get(&(user_id.to_owned(), server_id.to_owned()))
            .cloned()
    }

    pub(crate) fn merges(&self) -> Vec<MergeCall> {
        self.merges.lock().unwrap().clone()
    }

    pub(crate) fn fail_reads(&self) {
        self.fail_get.store(true, Ordering::SeqCst);
    }

    pub(crate) fn fail_bulk_read(&self) {
        self.fail_bulk.store(true, Ordering::SeqCst);
    }

    pub(crate) fn fail_write(&self) {
        self.fail_merge.store(true, Ordering::SeqCst);
    }

    pub(crate) fn fail_clear(&self) {
        self.fail_delete.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl UserEnvVarsStore for FakeStore {
    async fn get_user_env_vars(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<UserEnvVars, UserEnvVarsError> {
        self.journal.record(format!("get:{user_id}:{server_id}"));
        if self.fail_get.load(Ordering::SeqCst) {
            return Err(UserEnvVarsError::NotText);
        }
        Ok(self.row(user_id, server_id).unwrap_or_default())
    }

    async fn get_user_env_vars_bulk(
        &self,
        user_id: &str,
        server_ids: &[String],
    ) -> Result<BTreeMap<String, UserEnvVars>, UserEnvVarsError> {
        self.journal
            .record(format!("bulk:{user_id}:{}", server_ids.join(",")));
        if self.fail_bulk.load(Ordering::SeqCst) {
            return Err(UserEnvVarsError::NotText);
        }
        let rows = self.rows.lock().unwrap();
        Ok(server_ids
            .iter()
            .filter_map(|server_id| {
                let key = (user_id.to_owned(), server_id.clone());
                rows.get(&key)
                    .map(|values| (server_id.clone(), values.clone()))
            })
            .collect())
    }

    async fn merge_user_env_vars(
        &self,
        user_id: &str,
        server_id: &str,
        updates: &UserEnvVars,
        allowed_names: &BTreeSet<String>,
    ) -> Result<UserEnvVars, UserEnvVarsError> {
        self.journal.record(format!("merge:{user_id}:{server_id}"));
        self.merges.lock().unwrap().push(MergeCall {
            user_id: user_id.to_owned(),
            server_id: server_id.to_owned(),
            updates: updates.clone(),
            allowed_names: allowed_names.clone(),
        });
        if self.fail_merge.load(Ordering::SeqCst) {
            return Err(UserEnvVarsError::NotText);
        }
        let mut rows = self.rows.lock().unwrap();
        let values = rows
            .entry((user_id.to_owned(), server_id.to_owned()))
            .or_default();
        values.retain(|name, _| allowed_names.contains(name));
        values.extend(
            updates
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
        Ok(values.clone())
    }

    async fn delete_user_env_vars(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<(), UserEnvVarsError> {
        self.journal.record(format!("delete:{user_id}:{server_id}"));
        if self.fail_delete.load(Ordering::SeqCst) {
            return Err(UserEnvVarsError::NotText);
        }
        self.rows
            .lock()
            .unwrap()
            .remove(&(user_id.to_owned(), server_id.to_owned()));
        Ok(())
    }
}

pub(crate) struct RecordingCache {
    journal: Journal,
}

impl UserEnvVarsCache for RecordingCache {
    fn invalidate(&self, user_id: &str, server_id: &str) {
        self.journal.record(format!("cache:{user_id}:{server_id}"));
    }
}

#[derive(Clone, Copy)]
pub(crate) enum LookupAnswer {
    Found(&'static str),
    Missing,
    Fails,
}

pub(crate) struct FakeDb {
    journal: Journal,
    answer: LookupAnswer,
}

impl DbLookup for FakeDb {
    fn db_table<'lookup>(
        &'lookup self,
        server_id: &'lookup str,
    ) -> LookupFuture<'lookup, Option<McpServerTable>> {
        self.journal.record(format!("lookup:{server_id}"));
        let result = match self.answer {
            LookupAnswer::Found(json) => Ok(Some(serde_json::from_str(json).unwrap())),
            LookupAnswer::Missing => Ok(None),
            LookupAnswer::Fails => Err(ResolveError::from(McpTableError::ServerNotFound {
                server_id: server_id.to_owned(),
            })),
        };
        Box::pin(ready(result))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the access check failed")]
pub(crate) struct AccessFailure;

#[derive(Clone, Copy)]
pub(crate) enum AccessAnswer {
    Allows,
    Denies,
    Fails,
}

pub(crate) struct FakeAccess {
    journal: Journal,
    answer: AccessAnswer,
}

impl ServerAccessCheck for FakeAccess {
    type Error = AccessFailure;

    fn can_access(
        &self,
        _auth: &UserAPIKeyAuth,
        server_id: &str,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        self.journal.record(format!("access:{server_id}"));
        let result = match self.answer {
            AccessAnswer::Allows => Ok(true),
            AccessAnswer::Denies => Ok(false),
            AccessAnswer::Fails => Err(AccessFailure),
        };
        ready(result)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("source failure {number}")]
pub(crate) struct SourceFailure {
    number: u8,
}

#[derive(Clone)]
pub(crate) enum SourcesAnswer {
    Nothing,
    Servers(Vec<&'static str>),
    ContextsFail,
    TablesFail,
}

pub(crate) struct FakeSources {
    journal: Journal,
    answer: SourcesAnswer,
}

impl FakeSources {
    fn answer_tables(&self) -> Result<Vec<McpServerTable>, SourceFailure> {
        match &self.answer {
            SourcesAnswer::Nothing | SourcesAnswer::ContextsFail => Ok(Vec::new()),
            SourcesAnswer::Servers(jsons) => Ok(jsons
                .iter()
                .map(|json| serde_json::from_str(json).unwrap())
                .collect()),
            SourcesAnswer::TablesFail => Err(SourceFailure { number: 2 }),
        }
    }
}

impl AccessibleServerSources for FakeSources {
    type Error = SourceFailure;

    fn effective_auth_contexts(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> impl Future<Output = Result<Vec<UserAPIKeyAuth>, Self::Error>> + Send {
        self.journal.record("sources:contexts".to_owned());
        let result = match self.answer {
            SourcesAnswer::ContextsFail => Err(SourceFailure { number: 1 }),
            SourcesAnswer::Nothing => Ok(Vec::new()),
            SourcesAnswer::Servers(_) | SourcesAnswer::TablesFail => Ok(vec![auth.clone()]),
        };
        ready(result)
    }

    fn allowed_server_tables(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> impl Future<Output = Result<Vec<McpServerTable>, Self::Error>> + Send {
        self.journal.record("sources:tables".to_owned());
        ready(self.answer_tables())
    }
}

pub(crate) struct Harness {
    journal: Journal,
    store: Arc<FakeStore>,
    cache: Arc<RecordingCache>,
    database: bool,
    lookup: LookupAnswer,
    access: AccessAnswer,
    sources: SourcesAnswer,
    mode: ManagementMode,
    base_url: Option<String>,
    registry_servers: Vec<&'static str>,
}

impl Harness {
    pub(crate) fn new() -> Self {
        let journal = Journal::default();
        let store = Arc::new(FakeStore::new(journal.clone()));
        let cache = Arc::new(RecordingCache {
            journal: journal.clone(),
        });
        Self {
            journal,
            store,
            cache,
            database: true,
            lookup: LookupAnswer::Found(SERVER_ONE),
            access: AccessAnswer::Allows,
            sources: SourcesAnswer::Nothing,
            mode: ManagementMode::Restricted,
            base_url: Some("https://proxy.example".to_owned()),
            registry_servers: Vec::new(),
        }
    }

    pub(crate) fn without_database(mut self) -> Self {
        self.database = false;
        self
    }

    pub(crate) fn unknown_server(mut self) -> Self {
        self.lookup = LookupAnswer::Missing;
        self
    }

    pub(crate) fn for_server(mut self, json: &'static str) -> Self {
        self.lookup = LookupAnswer::Found(json);
        self
    }

    pub(crate) fn failing_lookup(mut self) -> Self {
        self.lookup = LookupAnswer::Fails;
        self
    }

    pub(crate) fn denying_access(mut self) -> Self {
        self.access = AccessAnswer::Denies;
        self
    }

    pub(crate) fn failing_access(mut self) -> Self {
        self.access = AccessAnswer::Fails;
        self
    }

    pub(crate) fn answering_from(mut self, sources: SourcesAnswer) -> Self {
        self.sources = sources;
        self
    }

    pub(crate) fn in_mode(mut self, mode: ManagementMode) -> Self {
        self.mode = mode;
        self
    }

    pub(crate) fn with_registry_servers(mut self, servers: Vec<&'static str>) -> Self {
        self.registry_servers = servers;
        self
    }

    pub(crate) fn without_base_url(mut self) -> Self {
        self.base_url = None;
        self
    }

    pub(crate) fn store(&self) -> &FakeStore {
        &self.store
    }

    pub(crate) fn journal(&self) -> Vec<String> {
        self.journal.entries()
    }

    fn registry(&self) -> SharedRegistry {
        let settings = RegistrySettings {
            public_mcp_servers: Vec::new(),
            public_hub_strict: true,
            internal_ip_ranges: None,
        };
        let config_servers = self
            .registry_servers
            .iter()
            .map(|json| serde_json::from_str(json).unwrap())
            .collect();
        Arc::new(RwLock::new(ServerRegistry::new(
            settings,
            config_servers,
            Vec::new(),
        )))
    }

    pub(crate) fn router(&self) -> Router {
        let store: Arc<dyn UserEnvVarsStore> = Arc::clone(&self.store);
        let cache: Arc<dyn UserEnvVarsCache> = Arc::clone(&self.cache);
        let state = UserEnvRoutesState {
            database: self
                .database
                .then(|| Arc::new(PostgresStore { pool: None })),
            store,
            cache,
            registry: self.registry(),
            db_lookup: Arc::new(FakeDb {
                journal: self.journal.clone(),
                answer: self.lookup,
            }),
            access: Arc::new(FakeAccess {
                journal: self.journal.clone(),
                answer: self.access,
            }),
            sources: Arc::new(FakeSources {
                journal: self.journal.clone(),
                answer: self.sources.clone(),
            }),
            mode: self.mode,
            proxy_base_url: self.base_url.clone(),
        };
        routes(state)
    }
}

pub(crate) fn get(uri: &str, caller: Option<&AuthContext>) -> Request<Body> {
    request("GET", uri, caller, None)
}

pub(crate) fn delete(uri: &str, caller: Option<&AuthContext>) -> Request<Body> {
    request("DELETE", uri, caller, None)
}

pub(crate) fn put(uri: &str, caller: Option<&AuthContext>, body: &str) -> Request<Body> {
    request(
        "PUT",
        uri,
        caller,
        Some((Some("application/json"), body.as_bytes())),
    )
}

pub(crate) fn post(uri: &str, caller: Option<&AuthContext>, body: &str) -> Request<Body> {
    request(
        "POST",
        uri,
        caller,
        Some((Some("application/json"), body.as_bytes())),
    )
}

pub(crate) fn post_raw(
    uri: &str,
    caller: Option<&AuthContext>,
    content_type: Option<&str>,
    body: &[u8],
) -> Request<Body> {
    request("POST", uri, caller, Some((content_type, body)))
}

fn request(
    method: &str,
    uri: &str,
    caller: Option<&AuthContext>,
    posted: Option<(Option<&str>, &[u8])>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some((Some(content_type), _)) = posted {
        builder = builder.header("content-type", content_type);
    }
    let mut built = builder
        .body(posted.map_or_else(Body::empty, |(_, bytes)| Body::from(bytes.to_vec())))
        .unwrap();
    if let Some(context) = caller {
        built.extensions_mut().insert(context.clone());
    }
    built
}

pub(crate) async fn call(app: Router, request: Request<Body>) -> Reply {
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    Reply {
        status,
        body: bytes.to_vec(),
    }
}

pub(crate) struct Reply {
    status: StatusCode,
    body: Vec<u8>,
}

impl Reply {
    pub(crate) fn status(&self) -> StatusCode {
        self.status
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.body.clone()).unwrap()
    }

    pub(crate) fn json<T>(&self) -> T
    where
        T: DeserializeOwned,
    {
        serde_json::from_slice(&self.body).unwrap()
    }

    pub(crate) fn detail(&self) -> BTreeMap<String, String> {
        self.json::<TextMapDetail>().detail
    }

    pub(crate) fn text_detail(&self) -> String {
        self.json::<TextDetail>().detail
    }

    pub(crate) fn issues(&self) -> Vec<TestIssue> {
        self.json::<IssuesDetail>().detail
    }

    pub(crate) fn error(&self) -> ErrorPayload {
        self.json::<ErrorEnvelope>().error
    }
}

#[derive(Deserialize)]
struct TextMapDetail {
    detail: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct TextDetail {
    detail: String,
}

#[derive(Deserialize)]
struct IssuesDetail {
    detail: Vec<TestIssue>,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct TestIssue {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) loc: Vec<String>,
    pub(crate) msg: String,
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorPayload,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorPayload {
    pub(crate) message: String,
    #[serde(rename = "type")]
    pub(crate) kind: String,
}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}
