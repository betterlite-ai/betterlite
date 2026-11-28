use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_cache::backend::{BackendError, CacheBackend, CacheResult, Ttl};
use betterlite_config::sections::mcp_env_values::PayloadRules;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp::table_access::types::McpTableError;
use betterlite_db::mcp_server_encoding::request::McpServerRequest;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::registry::resolution::ServerAccessCheck;
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry, SharedRegistry};
use betterlite_mcp::server_build::BuildError;
use betterlite_mcp::server_payload::request::NewMCPServerRequest;
use betterlite_mcp::server_record::MCPServer;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, OsNonces, StoredCipher, StoredValue,
};
use chrono::{DateTime, FixedOffset, TimeDelta, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::RwLock;

use crate::failure_hook::ports::Clock;
use crate::mcp::oauth_sessions::route::SessionRoute;
use crate::mcp::oauth_sessions::store::ports::{ServerFromTable, SessionDatabase, SessionIds};
use crate::mcp::oauth_sessions::store::{SessionStore, SessionStoreParts};
use crate::mcp::server_writes::{DraftOptions, ServerWriteError};

pub(crate) fn keyed_cipher(salt: &str) -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(salt.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(crate) fn test_cipher() -> DbCipher {
    keyed_cipher("test-salt")
}

pub(crate) fn keyless_cipher() -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: None,
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(crate) fn seal(cipher: &DbCipher, plaintext: &str) -> String {
    let stored = StoredValue::<String>::Text(SecretString::from(plaintext.to_owned()));
    let sealed = cipher
        .encrypt_value(stored, None)
        .expect("the test cipher seals the fixture");
    match sealed {
        StoredValue::Text(text) => text.expose_secret().to_owned(),
        StoredValue::Other(value) => value,
    }
}

pub(crate) fn server_named(server_id: &str, name: &str) -> MCPServer {
    let encoded = format!(
        r#"{{"server_id":"{server_id}","name":"{name}","transport":"sse","url":"https://mcp.example.test/sse"}}"#
    );
    serde_json::from_str(&encoded).expect("the test server decodes")
}

pub(crate) fn server(server_id: &str) -> MCPServer {
    server_named(server_id, server_id)
}

pub(crate) fn alias_server(server_id: &str, alias: &str, public: bool) -> MCPServer {
    let mut record = server(server_id);
    record.fields.alias = Some(alias.to_owned());
    record.fields.available_on_public_internet = public;
    record
}

pub(crate) fn row(server_id: &str) -> McpServerTable {
    row_named(server_id, server_id)
}

pub(crate) fn row_named(server_id: &str, server_name: &str) -> McpServerTable {
    let encoded = format!(
        r#"{{"server_id":"{server_id}","server_name":"{server_name}","transport":"sse","url":"https://mcp.example.test/sse"}}"#
    );
    serde_json::from_str(&encoded).expect("the test row decodes")
}

pub(crate) fn draft_row(server_id: &str) -> McpServerTable {
    let encoded = format!(
        r#"{{"server_id":"{server_id}","server_name":"{server_id}","transport":"sse","approval_status":"draft"}}"#
    );
    serde_json::from_str(&encoded).expect("the draft row decodes")
}

pub(crate) fn request(body: &str) -> NewMCPServerRequest {
    serde_json::from_str(body).expect("the test payload decodes")
}

pub(crate) fn auth_with(user_id: Option<&str>, role: &str) -> UserAPIKeyAuth {
    let encoded = match user_id {
        Some(user_id) => format!(r#"{{"user_id":"{user_id}","user_role":"{role}"}}"#),
        None => format!(r#"{{"user_role":"{role}"}}"#),
    };
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_str(&encoded).expect("the key fixture parses");
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the key fixture constructs")
}

pub(crate) fn now() -> DateTime<Utc> {
    DateTime::from_timestamp_millis(1_000_000).expect("the fixture instant is representable")
}

pub(crate) struct FakeClock {
    now: Mutex<DateTime<Utc>>,
}

impl FakeClock {
    pub(crate) fn new() -> Self {
        Self {
            now: Mutex::new(now()),
        }
    }

    pub(crate) fn advance(&self, seconds: i64) {
        let delta = TimeDelta::try_seconds(seconds).expect("the delta is representable");
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now
            .checked_add_signed(delta)
            .expect("the clock stays representable");
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<FixedOffset> {
        let moment = *self.now.lock().unwrap_or_else(PoisonError::into_inner);
        moment.fixed_offset()
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum RedisFault {
    #[default]
    None,
    Read,
    Write,
}

pub(crate) struct FakeRedis {
    values: Mutex<BTreeMap<String, CacheValue>>,
    ttls: Mutex<BTreeMap<String, Ttl>>,
    fault: RedisFault,
}

impl FakeRedis {
    pub(crate) fn new(fault: RedisFault) -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
            ttls: Mutex::new(BTreeMap::new()),
            fault,
        }
    }

    pub(crate) fn put(&self, key: &str, value: CacheValue) {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.insert(key.to_owned(), value);
    }

    pub(crate) fn get(&self, key: &str) -> Option<CacheValue> {
        let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.get(key).cloned()
    }

    pub(crate) fn clear(&self) {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.clear();
    }

    pub(crate) fn ttl(&self, key: &str) -> Option<f64> {
        let ttls = self.ttls.lock().unwrap_or_else(PoisonError::into_inner);
        ttls.get(key).copied().flatten()
    }

    pub(crate) fn len(&self) -> usize {
        let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.len()
    }
}

#[async_trait]
impl CacheBackend for FakeRedis {
    fn default_ttl(&self) -> i64 {
        60
    }

    async fn async_get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        if self.fault == RedisFault::Read {
            return Err(BackendError::Unsupported("async_get_cache"));
        }
        Ok(self.get(key))
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        if self.fault == RedisFault::Write {
            return Err(BackendError::Unsupported("async_set_cache"));
        }
        self.put(key, value);
        let mut ttls = self.ttls.lock().unwrap_or_else(PoisonError::into_inner);
        ttls.insert(key.to_owned(), ttl);
        Ok(())
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum DatabaseFault {
    #[default]
    None,
    ServerRead,
    DraftRead,
    DraftWrite,
}

pub(crate) struct FakeDatabase {
    servers: Mutex<BTreeMap<String, McpServerTable>>,
    drafts: Mutex<BTreeMap<String, McpServerTable>>,
    created: Mutex<Vec<(McpServerRequest, DraftOptions)>>,
    fault: DatabaseFault,
}

impl FakeDatabase {
    pub(crate) fn new(fault: DatabaseFault) -> Self {
        Self {
            servers: Mutex::new(BTreeMap::new()),
            drafts: Mutex::new(BTreeMap::new()),
            created: Mutex::new(Vec::new()),
            fault,
        }
    }

    pub(crate) fn put_server(&self, row: McpServerTable) {
        let mut servers = self.servers.lock().unwrap_or_else(PoisonError::into_inner);
        servers.insert(row.server_id.clone(), row);
    }

    pub(crate) fn put_draft(&self, row: McpServerTable) {
        let mut drafts = self.drafts.lock().unwrap_or_else(PoisonError::into_inner);
        drafts.insert(row.server_id.clone(), row);
    }

    pub(crate) fn remove_draft(&self, server_id: &str) {
        let mut drafts = self.drafts.lock().unwrap_or_else(PoisonError::into_inner);
        drafts.remove(server_id);
    }

    pub(crate) fn created(&self) -> Vec<(McpServerRequest, DraftOptions)> {
        let created = self.created.lock().unwrap_or_else(PoisonError::into_inner);
        created.clone()
    }
}

impl SessionDatabase for FakeDatabase {
    fn get_server<'db>(
        &'db self,
        server_id: &'db str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<McpServerTable>, McpTableError>> + Send + 'db>>
    {
        let fault = self.fault;
        let row = {
            let servers = self.servers.lock().unwrap_or_else(PoisonError::into_inner);
            servers.get(server_id).cloned()
        };
        Box::pin(async move {
            if fault == DatabaseFault::ServerRead {
                return Err(McpTableError::MissingServerId);
            }
            Ok(row)
        })
    }

    fn get_draft<'db>(
        &'db self,
        server_id: &'db str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<McpServerTable>, ServerWriteError>> + Send + 'db>>
    {
        let fault = self.fault;
        let row = {
            let drafts = self.drafts.lock().unwrap_or_else(PoisonError::into_inner);
            drafts.get(server_id).cloned()
        };
        Box::pin(async move {
            if fault == DatabaseFault::DraftRead {
                return Err(ServerWriteError::MissingServerId);
            }
            Ok(row)
        })
    }

    fn create_draft<'db>(
        &'db self,
        request: &'db McpServerRequest,
        options: &'db DraftOptions,
    ) -> Pin<Box<dyn Future<Output = Result<McpServerTable, ServerWriteError>> + Send + 'db>> {
        let fault = self.fault;
        let recorded = (request.clone(), options.clone());
        let draft_id = options
            .server_id
            .clone()
            .or_else(|| request.server_id.clone())
            .unwrap_or_else(|| "draft-written".to_owned());
        if fault != DatabaseFault::DraftWrite {
            let mut created = self.created.lock().unwrap_or_else(PoisonError::into_inner);
            created.push(recorded);
        }
        Box::pin(async move {
            if fault == DatabaseFault::DraftWrite {
                return Err(ServerWriteError::InsertedRowMissing);
            }
            Ok(draft_row(&draft_id))
        })
    }
}

pub(crate) struct FakeBuilder {
    calls: Mutex<Vec<(String, bool)>>,
    failing: bool,
}

impl FakeBuilder {
    pub(crate) fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failing: false,
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failing: true,
        }
    }

    pub(crate) fn calls(&self) -> Vec<(String, bool)> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.clone()
    }
}

impl ServerFromTable for FakeBuilder {
    fn build_server<'build>(
        &'build self,
        table: &'build McpServerTable,
        credentials_are_encrypted: bool,
    ) -> Pin<Box<dyn Future<Output = Result<MCPServer, BuildError>> + Send + 'build>> {
        let failing = self.failing;
        let server = server_named(
            &table.server_id,
            table.server_name.as_deref().unwrap_or(&table.server_id),
        );
        {
            let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
            calls.push((table.server_id.clone(), credentials_are_encrypted));
        }
        Box::pin(async move {
            if failing {
                return Err(BuildError::PrefixExhausted {
                    server_id: server.fields.server_id.clone(),
                    attempts: 3,
                });
            }
            Ok(server)
        })
    }
}

pub(crate) struct FakeIds {
    next: Mutex<u32>,
}

impl FakeIds {
    pub(crate) fn new() -> Self {
        Self {
            next: Mutex::new(1),
        }
    }
}

impl SessionIds for FakeIds {
    fn fresh_id(&self) -> String {
        let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
        let current = *next;
        *next = next.saturating_add(1);
        format!("fresh-session-{current}")
    }
}

#[derive(Clone, Copy, Debug, Default, thiserror::Error)]
#[error("the access check is unavailable")]
pub(crate) struct AccessFailure;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum AccessOutcome {
    #[default]
    Allow,
    Deny,
    Fail,
}

pub(crate) struct FakeAccess {
    outcome: AccessOutcome,
}

impl FakeAccess {
    pub(crate) fn new(outcome: AccessOutcome) -> Self {
        Self { outcome }
    }
}

impl ServerAccessCheck for FakeAccess {
    type Error = AccessFailure;

    fn can_access(
        &self,
        _auth: &UserAPIKeyAuth,
        _server_id: &str,
    ) -> impl Future<Output = Result<bool, AccessFailure>> + Send {
        let outcome = self.outcome;
        ready(match outcome {
            AccessOutcome::Allow => Ok(true),
            AccessOutcome::Deny => Ok(false),
            AccessOutcome::Fail => Err(AccessFailure),
        })
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum CipherChoice {
    #[default]
    Working,
    Keyless,
}

#[derive(Default)]
pub(crate) struct HarnessConfig {
    pub(crate) redis: Option<RedisFault>,
    pub(crate) database: Option<DatabaseFault>,
    pub(crate) cipher: CipherChoice,
    pub(crate) access: AccessOutcome,
    pub(crate) builder: BuilderChoice,
    pub(crate) servers: Vec<MCPServer>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum BuilderChoice {
    #[default]
    Working,
    Failing,
}

impl HarnessConfig {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_redis(self, fault: RedisFault) -> Self {
        Self {
            redis: Some(fault),
            ..self
        }
    }

    pub(crate) fn with_database(self, fault: DatabaseFault) -> Self {
        Self {
            database: Some(fault),
            ..self
        }
    }

    pub(crate) fn with_cipher(self, cipher: CipherChoice) -> Self {
        Self { cipher, ..self }
    }

    pub(crate) fn with_access(self, access: AccessOutcome) -> Self {
        Self { access, ..self }
    }

    pub(crate) fn with_builder(self, builder: BuilderChoice) -> Self {
        Self { builder, ..self }
    }

    pub(crate) fn with_servers(self, servers: Vec<MCPServer>) -> Self {
        Self { servers, ..self }
    }
}

pub(crate) fn temporary_key(server_id: &str) -> String {
    format!("litellm:mcp:temporary_server:{server_id}")
}

pub(crate) fn sealed_server(cipher: &DbCipher, server: &MCPServer) -> String {
    let encoded = serde_json::to_string(server).expect("the fixture server encodes");
    seal(cipher, &encoded)
}

pub(crate) struct Harness {
    pub(crate) clock: Arc<FakeClock>,
    pub(crate) cipher: Arc<DbCipher>,
    pub(crate) redis: Option<Arc<FakeRedis>>,
    pub(crate) database: Option<Arc<FakeDatabase>>,
    pub(crate) builder: Arc<FakeBuilder>,
    pub(crate) store: Arc<SessionStore<FakeAccess>>,
    pub(crate) registry: SharedRegistry,
}

struct Fakes {
    clock: Arc<FakeClock>,
    cipher: Arc<DbCipher>,
    redis: Option<Arc<FakeRedis>>,
    database: Option<Arc<FakeDatabase>>,
    builder: Arc<FakeBuilder>,
}

fn harness_fakes(config: &HarnessConfig) -> Fakes {
    let clock = Arc::new(FakeClock::new());
    let cipher = Arc::new(match config.cipher {
        CipherChoice::Working => test_cipher(),
        CipherChoice::Keyless => keyless_cipher(),
    });
    let redis = match config.redis {
        Some(fault) => Some(Arc::new(FakeRedis::new(fault))),
        None => None,
    };
    let database = match config.database {
        Some(fault) => Some(Arc::new(FakeDatabase::new(fault))),
        None => None,
    };
    let builder = Arc::new(match config.builder {
        BuilderChoice::Working => FakeBuilder::new(),
        BuilderChoice::Failing => FakeBuilder::failing(),
    });
    Fakes {
        clock,
        cipher,
        redis,
        database,
        builder,
    }
}

fn harness_store(
    fakes: &Fakes,
    registry: &SharedRegistry,
    access: AccessOutcome,
) -> Arc<SessionStore<FakeAccess>> {
    let ids = Arc::new(FakeIds::new());
    Arc::new(SessionStore::new(SessionStoreParts {
        clock: as_clock(&fakes.clock),
        cipher: Arc::<DbCipher>::clone(&fakes.cipher),
        redis: fakes.redis.as_ref().map(as_redis),
        database: fakes.database.as_ref().map(as_database),
        builder: as_builder(&fakes.builder),
        ids: as_ids(&ids),
        access: FakeAccess::new(access),
    }))
}

pub(crate) fn harness(config: HarnessConfig) -> Harness {
    let fakes = harness_fakes(&config);
    let registry = Arc::new(RwLock::new(ServerRegistry::new(
        RegistrySettings {
            public_mcp_servers: Vec::new(),
            public_hub_strict: false,
            internal_ip_ranges: None,
        },
        config.servers,
        Vec::new(),
    )));
    let store = harness_store(&fakes, &registry, config.access);
    Harness {
        clock: fakes.clock,
        cipher: fakes.cipher,
        redis: fakes.redis,
        database: fakes.database,
        builder: fakes.builder,
        store,
        registry,
    }
}

pub(crate) fn session_route(harness: &Harness) -> SessionRoute<FakeAccess> {
    SessionRoute {
        store: Arc::clone(&harness.store),
        registry: Arc::clone(&harness.registry),
        rules: PayloadRules::default(),
    }
}

pub(crate) fn as_clock(shared: &Arc<FakeClock>) -> Arc<dyn Clock> {
    Arc::<FakeClock>::clone(shared)
}

pub(crate) fn as_redis(shared: &Arc<FakeRedis>) -> Arc<dyn CacheBackend> {
    Arc::<FakeRedis>::clone(shared)
}

pub(crate) fn as_database(shared: &Arc<FakeDatabase>) -> Arc<dyn SessionDatabase> {
    Arc::<FakeDatabase>::clone(shared)
}

pub(crate) fn as_builder(shared: &Arc<FakeBuilder>) -> Arc<dyn ServerFromTable> {
    Arc::<FakeBuilder>::clone(shared)
}

pub(crate) fn as_ids(shared: &Arc<FakeIds>) -> Arc<dyn SessionIds> {
    Arc::<FakeIds>::clone(shared)
}
