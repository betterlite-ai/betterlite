use std::collections::BTreeMap;
use std::future::{Future, Ready, ready};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::mcp::table_access::types::McpTableError;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_stored_contracts::MCPAuth;
use betterlite_db::mcp_stored_contracts::secret::StoredSecret;
use betterlite_db::mcp_user_credentials::rows::{CredentialRows, CredentialScope};
use betterlite_db::oauth_token_resolution::ports::TokenResponse;
use betterlite_db::oauth_token_resolution::{EnforceRequest, OauthBindingPort};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_http::proxy::ProxiedClient;
use betterlite_mcp::oauth_identity_binding::port_types::{BindingDenied, RefreshOwnership};
use betterlite_mcp::server_record::binding::{BindingMode, CallerField};
use betterlite_mcp::server_record::{MCPOAuthIdentityBinding, MCPServer, SecretCredential};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, OsNonces, StoredCipher, StoredValue,
};
use chrono::{DateTime, Utc};
use http::StatusCode;
use secrecy::{ExposeSecret, SecretString};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinSet;

use super::super::super::authz_code_refresher::ports::{
    CredentialPersist, PersistRequest, ServerLookup, TokenEndpointPost,
};
use super::super::super::cache_codec::OAuthTokenCacheCodec;
use super::super::super::dual_cache::AsyncCache;
use super::super::super::v2_store::{CredentialPayload, CredentialReader, CredentialValue};
use super::super::super::{
    InvalidatableOAuthTokenStore, OAuthToken, OAuthTokenStore, RefreshCoordinator, RefreshStep,
    TokenKey, TokenStoreUnavailable, WallClock,
};
use super::super::adapters::BoundCredentialReader;
use super::super::chain::{BuiltStore, PerUserStoreBuilder, RedisChain, RedisParts, StoreBuilder};
use super::super::lazy::{LazyPerUserOAuthTokenStore, RedisAvailability};

pub(super) const DEADLINE: Duration = Duration::from_secs(5);

pub(super) fn test_cipher() -> Arc<DbCipher> {
    Arc::new(StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("b4660-salt".to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    })
}

pub(super) fn base_time() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2024-01-02T03:04:05+00:00")
        .unwrap()
        .with_timezone(&Utc)
}

pub(super) fn exposed(token: Option<&SecretString>) -> Option<&str> {
    token.map(ExposeSecret::expose_secret)
}

pub(super) fn sealed_byok(cipher: &DbCipher) -> String {
    let value = StoredValue::<()>::Text(SecretString::from("byok".to_owned()));
    let StoredValue::Text(sealed) = cipher.encrypt_value(value, None).unwrap() else {
        panic!("a text value seals as text");
    };
    sealed.expose_secret().to_owned()
}

pub(super) fn pooled_client() -> Arc<ProxiedClient> {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    Arc::new(ProxiedClient::outbound(&policy).unwrap())
}

pub(super) fn answer_with_object() -> Ready<(StatusCode, String)> {
    ready((StatusCode::OK, String::from("{\"access_token\":\"fresh\"}")))
}

pub(super) fn answer_with_array() -> Ready<(StatusCode, String)> {
    ready((StatusCode::OK, String::from("[1, 2]")))
}

pub(super) fn answer_with_server_error() -> Ready<(StatusCode, String)> {
    ready((
        StatusCode::INTERNAL_SERVER_ERROR,
        String::from("{\"access_token\":\"fresh\"}"),
    ))
}

pub(super) async fn serving_token_endpoint(application: Router) -> (String, JoinSet<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        axum::serve(listener, application).await.unwrap();
    });
    (format!("http://{address}/token"), tasks)
}

pub(super) struct FixedWallClock(pub(super) DateTime<Utc>);

impl WallClock for FixedWallClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

#[derive(Clone, Default)]
pub(super) struct FakeRows {
    pub(super) sealed: Arc<Mutex<Option<String>>>,
}

impl FakeRows {
    pub(super) fn sealed(credential: String) -> Self {
        Self {
            sealed: Arc::new(Mutex::new(Some(credential))),
        }
    }
}

impl CredentialRows for FakeRows {
    fn find_credential(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send {
        let sealed = self.sealed.lock().unwrap().clone();
        ready(Ok(sealed.map(|credential| {
            JsonData::Object(BTreeMap::from([
                ("user_id".to_owned(), JsonData::String(user_id.to_owned())),
                (
                    "server_id".to_owned(),
                    JsonData::String(server_id.to_owned()),
                ),
                ("credential_b64".to_owned(), JsonData::String(credential)),
            ]))
        })))
    }

    fn find_credentials(
        &self,
        _scope: CredentialScope<'_>,
    ) -> impl Future<Output = Result<Vec<JsonData>, McpTableError>> + Send {
        ready(Ok(Vec::new()))
    }

    // the trait's two writers share one signature; one line each keeps them under the similarity check's 8-line floor
    #[rustfmt::skip]
    fn upsert_credential(&self, _user_id: &str, _server_id: &str, credential_b64: &str) -> impl Future<Output = Result<(), McpTableError>> + Send {
        *self.sealed.lock().unwrap() = Some(credential_b64.to_owned());
        ready(Ok(()))
    }

    #[rustfmt::skip]
    fn delete_credential(&self, _user_id: &str, _server_id: &str) -> impl Future<Output = Result<(), McpTableError>> + Send {
        drop(self.sealed.lock().unwrap().take());
        ready(Ok(()))
    }
}

pub(super) struct FailingRows;

impl CredentialRows for FailingRows {
    fn find_credential(
        &self,
        _user_id: &str,
        _server_id: &str,
    ) -> impl Future<Output = Result<Option<JsonData>, McpTableError>> + Send {
        ready(Err(McpTableError::MissingServerId))
    }

    fn find_credentials(
        &self,
        _scope: CredentialScope<'_>,
    ) -> impl Future<Output = Result<Vec<JsonData>, McpTableError>> + Send {
        ready(Ok(Vec::new()))
    }

    // the trait's two writers share one signature; one line each keeps them under the similarity check's 8-line floor
    #[rustfmt::skip]
    fn upsert_credential(&self, _user_id: &str, _server_id: &str, _credential_b64: &str) -> impl Future<Output = Result<(), McpTableError>> + Send {
        ready(Ok(()))
    }

    #[rustfmt::skip]
    fn delete_credential(&self, _user_id: &str, _server_id: &str) -> impl Future<Output = Result<(), McpTableError>> + Send {
        ready(Ok(()))
    }
}

fn credential(proof: Option<&str>) -> CredentialPayload {
    CredentialPayload {
        access_token: CredentialValue::Text("stored".to_owned()),
        refresh_token: CredentialValue::Null,
        expires_at: CredentialValue::Null,
        scopes: CredentialValue::Null,
        identity_binding_proof: proof.map_or(CredentialValue::Null, |text| {
            CredentialValue::Text(text.to_owned())
        }),
    }
}

pub(super) fn oauth_server(binding_mode: Option<&str>) -> Arc<MCPServer> {
    let mut server: MCPServer =
        serde_json::from_str(r#"{"server_id":"srv","name":"Server","transport":"http"}"#).unwrap();
    server.fields.auth_type = Some(MCPAuth::Oauth2);
    server.fields.client_id = Some("client".to_owned());
    server.fields.token_url = Some("https://idp.test/token".to_owned());
    server.fields.client_secret = Some(SecretCredential(StoredSecret(SecretString::from(
        "secret".to_owned(),
    ))));
    server.fields.oauth_identity_binding = binding_mode.map(|mode| MCPOAuthIdentityBinding {
        mode: if mode == "audit" {
            BindingMode::Audit
        } else {
            BindingMode::Enforce
        },
        issuer: "https://idp.test".to_owned(),
        jwks_url: None,
        audiences: vec!["aud".to_owned()],
        principal_claim: "email".to_owned(),
        caller_field: CallerField::UserEmail,
        require_email_verified: true,
    });
    Arc::new(server)
}

pub(super) struct FakeLookup {
    pub(super) server: Option<Arc<MCPServer>>,
}

impl ServerLookup for FakeLookup {
    fn server(&self, _server_id: &str) -> Option<Arc<MCPServer>> {
        self.server.clone()
    }
}

pub(super) struct RecordingBinding {
    pub(super) matches: bool,
    pub(super) checks: AtomicUsize,
    pub(super) proof: Option<String>,
    pub(super) proofs: Mutex<Vec<Option<String>>>,
}

impl OauthBindingPort for RecordingBinding {
    type Server = MCPServer;
    type Ownership = RefreshOwnership;
    type Denied = BindingDenied;

    fn enforce(
        &self,
        _request: &EnforceRequest<'_, MCPServer, RefreshOwnership>,
    ) -> impl Future<Output = Result<Option<String>, BindingDenied>> + Send {
        ready(Ok(self.proof.clone()))
    }

    fn proof_matches(
        &self,
        _server: &MCPServer,
        _user_id: &str,
        stored_proof: Option<&str>,
    ) -> impl Future<Output = bool> + Send {
        self.checks.fetch_add(1, Ordering::Relaxed);
        self.proofs
            .lock()
            .unwrap()
            .push(stored_proof.map(str::to_owned));
        ready(self.matches)
    }
}

pub(super) struct FakeReader {
    credential: Option<CredentialPayload>,
}

#[async_trait]
impl CredentialReader for FakeReader {
    async fn read_credential(
        &self,
        _user_id: &str,
        _server_id: &str,
    ) -> Result<Option<CredentialPayload>, TokenStoreUnavailable> {
        Ok(self.credential.clone())
    }
}

pub(super) struct RecordingPost {
    answer: Option<TokenResponse>,
    pub(super) calls: AtomicUsize,
}

impl TokenEndpointPost for RecordingPost {
    fn post_form(
        &self,
        _url: &str,
        _form: &BTreeMap<String, String>,
        _headers: &BTreeMap<String, String>,
    ) -> impl Future<Output = Option<TokenResponse>> + Send {
        self.calls.fetch_add(1, Ordering::Relaxed);
        ready(self.answer.clone())
    }
}

pub(super) struct RecordingPersist {
    pub(super) calls: AtomicUsize,
}

impl CredentialPersist for RecordingPersist {
    fn persist(
        &self,
        _request: &PersistRequest<'_>,
    ) -> impl Future<Output = Result<(), TokenStoreUnavailable>> + Send {
        self.calls.fetch_add(1, Ordering::Relaxed);
        ready(Ok(()))
    }
}

pub(super) struct ReaderFixture {
    pub(super) reader: BoundCredentialReader<FakeLookup, RecordingBinding, FakeReader>,
    pub(super) binding: Arc<RecordingBinding>,
}

pub(super) fn reader_fixture(
    server: Option<Arc<MCPServer>>,
    proof: Option<&str>,
    matches: bool,
) -> ReaderFixture {
    let binding = Arc::new(RecordingBinding {
        matches,
        checks: AtomicUsize::new(0),
        proof: None,
        proofs: Mutex::new(Vec::new()),
    });
    let reader = BoundCredentialReader {
        server_lookup: Arc::new(FakeLookup { server }),
        binding: Arc::clone(&binding),
        reader: Arc::new(FakeReader {
            credential: Some(credential(proof)),
        }),
    };
    ReaderFixture { reader, binding }
}

pub(super) fn expired_credential() -> CredentialPayload {
    CredentialPayload {
        access_token: CredentialValue::Text("stale".to_owned()),
        refresh_token: CredentialValue::Text("stored".to_owned()),
        expires_at: CredentialValue::Text("2000-01-01T00:00:00+00:00".to_owned()),
        scopes: CredentialValue::Array(vec![CredentialValue::Text("read".to_owned())]),
        identity_binding_proof: CredentialValue::Null,
    }
}

pub(super) fn grant(expires_in: Option<i64>) -> TokenResponse {
    let mut grant = TokenResponse::from([
        (
            "access_token".to_owned(),
            JsonValue::String("fresh".to_owned()),
        ),
        (
            "refresh_token".to_owned(),
            JsonValue::String("rotated".to_owned()),
        ),
    ]);
    if let Some(seconds) = expires_in {
        drop(grant.insert(
            "expires_in".to_owned(),
            JsonValue::Number(serde_json::Number::from(seconds)),
        ));
        drop(grant.insert("scope".to_owned(), JsonValue::String("read".to_owned())));
    }
    grant
}

pub(super) struct RecordingCoordinator {
    runs: Arc<AtomicUsize>,
}

#[async_trait]
impl RefreshCoordinator for RecordingCoordinator {
    type Output = Option<OAuthToken>;
    type Error = TokenStoreUnavailable;

    async fn run(
        &self,
        _key: TokenKey,
        refresh: RefreshStep<'_, Option<OAuthToken>, TokenStoreUnavailable>,
        _reread: RefreshStep<'_, Option<OAuthToken>, TokenStoreUnavailable>,
    ) -> Result<Option<OAuthToken>, TokenStoreUnavailable> {
        self.runs.fetch_add(1, Ordering::Relaxed);
        refresh().await
    }
}

type CacheEntries = BTreeMap<String, (String, Option<Duration>)>;

type MaybeTtl = Option<Option<Duration>>;

#[derive(Clone, Default)]
pub(super) struct FakeCache {
    entries: Arc<Mutex<CacheEntries>>,
}

impl FakeCache {
    pub(super) fn ttl_of(&self, key: &str) -> MaybeTtl {
        self.entries
            .lock()
            .unwrap()
            .get(key)
            .map(|(_value, ttl)| *ttl)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake cache is unavailable")]
pub(super) struct CacheUnavailable;

#[async_trait]
impl AsyncCache for FakeCache {
    type Error = CacheUnavailable;

    async fn async_get_cache(&self, key: &str) -> Result<Option<CacheValue>, CacheUnavailable> {
        let hit = self
            .entries
            .lock()
            .unwrap()
            .get(key)
            .map(|(value, _ttl)| CacheValue::Text(value.clone()));
        Ok(hit)
    }

    async fn async_set_cache(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), CacheUnavailable> {
        drop(
            self.entries
                .lock()
                .unwrap()
                .insert(key.to_owned(), (value.to_owned(), ttl)),
        );
        Ok(())
    }

    async fn async_delete_cache(&self, key: &str) -> Result<(), CacheUnavailable> {
        drop(self.entries.lock().unwrap().remove(key));
        Ok(())
    }
}

fn plain_codec() -> OAuthTokenCacheCodec {
    OAuthTokenCacheCodec {
        encrypt: Box::new(|value: &str| value.to_owned()),
        decrypt: Box::new(|blob: &str| Some(blob.to_owned())),
    }
}

pub(super) struct FakeRedis {
    wired: bool,
    cache: FakeCache,
    coordinator:
        Arc<dyn RefreshCoordinator<Output = Option<OAuthToken>, Error = TokenStoreUnavailable>>,
}

impl RedisChain for FakeRedis {
    type Cache = FakeCache;

    fn parts(&self) -> Option<RedisParts<FakeCache>> {
        self.wired.then(|| RedisParts {
            cache: self.cache.clone(),
            codec: plain_codec(),
            coordinator: Arc::clone(&self.coordinator),
        })
    }
}

pub(super) struct ChainFixture {
    pub(super) builder: PerUserStoreBuilder<
        FakeLookup,
        RecordingBinding,
        FakeReader,
        RecordingPersist,
        RecordingPost,
        FakeRedis,
    >,
    pub(super) persist: Arc<RecordingPersist>,
    pub(super) post: Arc<RecordingPost>,
    pub(super) cache: FakeCache,
    pub(super) runs: Arc<AtomicUsize>,
}

pub(super) fn chain_fixture(
    server: Option<Arc<MCPServer>>,
    credential: Option<CredentialPayload>,
    wired: bool,
    answer: Option<TokenResponse>,
) -> ChainFixture {
    let persist = Arc::new(RecordingPersist {
        calls: AtomicUsize::new(0),
    });
    let post = Arc::new(RecordingPost {
        answer,
        calls: AtomicUsize::new(0),
    });
    let cache = FakeCache::default();
    let runs = Arc::new(AtomicUsize::new(0));
    let builder = PerUserStoreBuilder {
        server_lookup: Arc::new(FakeLookup { server }),
        binding: Arc::new(RecordingBinding {
            matches: true,
            checks: AtomicUsize::new(0),
            proof: None,
            proofs: Mutex::new(Vec::new()),
        }),
        reader: Arc::new(FakeReader { credential }),
        persist: Arc::clone(&persist),
        post: Arc::clone(&post),
        redis: FakeRedis {
            wired,
            cache: cache.clone(),
            coordinator: Arc::new(RecordingCoordinator {
                runs: Arc::clone(&runs),
            }),
        },
    };
    ChainFixture {
        builder,
        persist,
        post,
        cache,
        runs,
    }
}

pub(super) struct FakeStore {
    token: Option<OAuthToken>,
    pub(super) fetches: AtomicUsize,
    pub(super) invalidations: Mutex<Vec<String>>,
    pub(super) started: Arc<Notify>,
    pub(super) release: Arc<Notify>,
    pub(super) hold_first: AtomicBool,
}

#[async_trait]
impl OAuthTokenStore for FakeStore {
    async fn fetch(
        &self,
        _user_id: &str,
        _server_id: &str,
    ) -> Result<Option<OAuthToken>, TokenStoreUnavailable> {
        self.fetches.fetch_add(1, Ordering::Relaxed);
        self.started.notify_one();
        if self.hold_first.swap(false, Ordering::SeqCst) {
            self.release.notified().await;
        }
        Ok(self.token.clone())
    }
}

#[async_trait]
impl InvalidatableOAuthTokenStore for FakeStore {
    async fn invalidate(&self, _user_id: &str, server_id: &str) {
        self.invalidations
            .lock()
            .unwrap()
            .push(server_id.to_owned());
    }
}

pub(super) struct FakeBuilder {
    store: Arc<FakeStore>,
    uses_redis: Arc<AtomicBool>,
    pub(super) builds: Arc<AtomicUsize>,
}

impl StoreBuilder for FakeBuilder {
    fn build(&self) -> BuiltStore {
        self.builds.fetch_add(1, Ordering::Relaxed);
        let fake_store: Arc<FakeStore> = Arc::clone(&self.store);
        let store: Arc<dyn InvalidatableOAuthTokenStore> = fake_store;
        BuiltStore {
            store,
            uses_redis: self.uses_redis.load(Ordering::Relaxed),
        }
    }
}

pub(super) struct FakeAvailability {
    available: Arc<AtomicBool>,
}

impl RedisAvailability for FakeAvailability {
    fn is_available(&self) -> bool {
        self.available.load(Ordering::Relaxed)
    }
}

pub(super) struct LazyFixture {
    pub(super) store:
        LazyPerUserOAuthTokenStore<FakeLookup, RecordingBinding, FakeBuilder, FakeAvailability>,
    pub(super) chain: Arc<FakeStore>,
    pub(super) builds: Arc<AtomicUsize>,
    pub(super) redis: Arc<AtomicBool>,
    pub(super) builder_redis: Arc<AtomicBool>,
}

pub(super) fn lazy_token(access: &str, proof: Option<&str>) -> OAuthToken {
    OAuthToken {
        access_token: access.to_owned(),
        expires_at: None,
        refresh_token: None,
        scopes: Vec::new(),
        identity_binding_proof: proof.map(str::to_owned),
    }
}

#[derive(Clone, Copy)]
pub(super) enum ChainWiring {
    Local,
    Redis,
}

pub(super) fn lazy_fixture(
    server: Option<Arc<MCPServer>>,
    token: Option<OAuthToken>,
    matches: bool,
    wiring: ChainWiring,
) -> LazyFixture {
    let chain = Arc::new(FakeStore {
        token,
        fetches: AtomicUsize::new(0),
        invalidations: Mutex::new(Vec::new()),
        started: Arc::new(Notify::new()),
        release: Arc::new(Notify::new()),
        hold_first: AtomicBool::new(false),
    });
    let builds = Arc::new(AtomicUsize::new(0));
    let redis = Arc::new(AtomicBool::new(false));
    let builder_redis = Arc::new(AtomicBool::new(matches!(wiring, ChainWiring::Redis)));
    let store = LazyPerUserOAuthTokenStore::new(
        Arc::new(FakeLookup { server }),
        Arc::new(RecordingBinding {
            matches,
            checks: AtomicUsize::new(0),
            proof: None,
            proofs: Mutex::new(Vec::new()),
        }),
        FakeBuilder {
            store: Arc::clone(&chain),
            uses_redis: Arc::clone(&builder_redis),
            builds: Arc::clone(&builds),
        },
        Arc::new(FakeAvailability {
            available: Arc::clone(&redis),
        }),
    );
    LazyFixture {
        store,
        chain,
        builds,
        redis,
        builder_redis,
    }
}
