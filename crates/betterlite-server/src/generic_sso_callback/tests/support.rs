use std::collections::BTreeMap;
use std::io;
use std::str;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse as _, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_auth::jwt_claims::handler::JwtHandler;
use betterlite_auth::jwt_claims::settings::JwtAuthSettings;
use betterlite_auth::sso_mappings::repository::SsoSettingsRows;
use betterlite_config::generic_sso_callback::{GenericSsoCallbackSettings, load_callback_settings};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sso_claims::{ClaimAttributes, load_claim_attributes};
use betterlite_config::sso_mappings::{
    AUTHORIZATION_ENDPOINT_ENV, CLIENT_SECRET_ENV, SsoMappingSources, TOKEN_ENDPOINT_ENV,
    USERINFO_ENDPOINT_ENV, load_mapping_sources,
};
use betterlite_config::sso_pkce::PkceSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::controls::json_value::JsonValue;
use secrecy::SecretString;
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::{MakeWriter, fmt};

use crate::generic_sso_callback::failure::GenericSsoError;
use crate::generic_sso_callback::{
    GenericSsoCallback, GenericSsoConfig, SsoExchangeResult, get_generic_sso_response,
};
use crate::sso_pkce::verifier_cache::{
    CacheRead, PkceCacheEntry, PkceCacheKind, PkceCaches, VerifierCache,
};

pub(super) const VERIFIER: &str = "verifier-for-state-1";
pub(super) const CLIENT_ID: &str = "generic-client";
pub(super) const REDIRECT_URI: &str = "https://proxy.test/sso/callback";

pub(super) fn sources(pairs: &[(&str, &str)]) -> SsoMappingSources {
    let environment: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    load_mapping_sources(&environment)
}

// the four required variables pointed at the local provider, with any test override on top
pub(super) fn mapping(base: &str, overrides: &[(&str, &str)]) -> SsoMappingSources {
    let authorize = format!("{base}/authorize");
    let token_endpoint = format!("{base}/token");
    let userinfo = format!("{base}/userinfo");
    let mut environment = BTreeMap::from([
        (CLIENT_SECRET_ENV.to_owned(), "generic-secret".to_owned()),
        (AUTHORIZATION_ENDPOINT_ENV.to_owned(), authorize),
        (TOKEN_ENDPOINT_ENV.to_owned(), token_endpoint),
        (USERINFO_ENDPOINT_ENV.to_owned(), userinfo),
    ]);
    for (name, value) in overrides {
        environment.insert((*name).to_owned(), (*value).to_owned());
    }
    load_mapping_sources(&environment)
}

pub(super) fn attributes(pairs: &[(&str, &str)]) -> ClaimAttributes {
    let environment: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    load_claim_attributes(&environment)
}

pub(super) fn callback_settings(pairs: &[(&str, &str)]) -> GenericSsoCallbackSettings {
    let environment: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    load_callback_settings(&environment)
}

pub(super) fn pkce(use_pkce: bool) -> PkceSettings {
    PkceSettings {
        use_pkce,
        strict_cache_miss: false,
        generic_client_state: None,
    }
}

pub(super) fn handler(settings: JwtAuthSettings) -> JwtHandler {
    let mut handler = JwtHandler::default();
    handler.update_environment(settings, Vec::new(), 0);
    handler
}

pub(super) fn team_handler(field: &str) -> JwtHandler {
    handler(JwtAuthSettings {
        team_id_jwt_field: None,
        team_ids_jwt_field: Some(field.to_owned()),
        team_id_default: None,
        team_alias_jwt_field: None,
        org_id_jwt_field: None,
        org_alias_jwt_field: None,
        user_id_jwt_field: None,
        user_email_jwt_field: None,
        user_allowed_email_domain: None,
        user_id_upsert: false,
        end_user_id_jwt_field: None,
        agent_id_jwt_field: None,
        object_id_jwt_field: None,
    })
}

pub(super) fn segment(raw: &str) -> String {
    URL_SAFE_NO_PAD.encode(raw.as_bytes())
}

pub(super) fn token(payload: &str) -> String {
    format!(
        "{}.{}.{}",
        segment(r#"{"alg":"HS256","typ":"JWT"}"#),
        segment(payload),
        segment("signature")
    )
}

pub(super) fn text(value: &str) -> JsonValue {
    JsonValue::Text(value.to_owned())
}

pub(super) struct FakeRows {
    reads: AtomicUsize,
}

impl FakeRows {
    pub(super) fn absent() -> Self {
        Self {
            reads: AtomicUsize::new(0),
        }
    }

    pub(super) fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl SsoSettingsRows for FakeRows {
    async fn stored_sso_settings(&self) -> Result<Option<String>, TableError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }
}

pub(super) struct FakeCache {
    entry: Option<PkceCacheEntry>,
    deletes: Mutex<Vec<String>>,
}

impl FakeCache {
    pub(super) fn empty() -> Self {
        Self {
            entry: None,
            deletes: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn holding_verifier() -> Self {
        Self {
            entry: Some(PkceCacheEntry::Verifier(SecretString::from(VERIFIER))),
            deletes: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn deletes(&self) -> Vec<String> {
        self.deletes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl VerifierCache for FakeCache {
    fn kind(&self) -> PkceCacheKind {
        PkceCacheKind::UserCache
    }

    async fn read(&self, _cache_key: &str) -> CacheRead {
        self.entry.clone().map_or(CacheRead::Miss, CacheRead::Found)
    }

    async fn delete(&self, cache_key: &str) {
        self.deletes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(cache_key.to_owned());
    }
}

fn caches(cache: &FakeCache) -> PkceCaches<'_, FakeCache, FakeCache> {
    PkceCaches {
        redis: None,
        user: cache,
    }
}

#[derive(Clone)]
pub(super) struct TokenReply {
    pub(super) access_token: String,
    pub(super) id_token: Option<String>,
    pub(super) refresh_token: Option<String>,
}

impl TokenReply {
    pub(super) fn opaque(access_token: &str) -> Self {
        Self {
            access_token: access_token.to_owned(),
            id_token: None,
            refresh_token: None,
        }
    }
}

pub(super) struct Replies {
    pub(super) token_status: u16,
    pub(super) token: TokenReply,
    pub(super) userinfo_status: u16,
    pub(super) userinfo: BTreeMap<String, JsonValue>,
}

impl Replies {
    pub(super) fn new() -> Self {
        Self {
            token_status: 200,
            token: TokenReply::opaque("access-opaque"),
            userinfo_status: 200,
            userinfo: BTreeMap::from([
                ("sub".to_owned(), text("subject-1")),
                ("preferred_username".to_owned(), text("user-1")),
                ("email".to_owned(), text("User@Example.test")),
            ]),
        }
    }
}

#[derive(Clone)]
pub(super) struct Seen {
    pub(super) method: &'static str,
    pub(super) path: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: String,
}

impl Seen {
    pub(super) fn form(&self, name: &str) -> String {
        url::form_urlencoded::parse(self.body.as_bytes())
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default()
    }

    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

#[derive(Clone)]
struct Provider {
    replies: Arc<Replies>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Provider {
    fn record(&self, method: &'static str, path: &str, headers: &HeaderMap, body: &str) {
        let captured = headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (name.as_str().to_owned(), text.to_owned()))
            })
            .collect();
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Seen {
                method,
                path: path.to_owned(),
                headers: captured,
                body: body.to_owned(),
            });
    }
}

pub(super) struct Running {
    pub(super) base: String,
    provider: Provider,
    tasks: JoinSet<()>,
}

impl Running {
    pub(super) fn requests(&self) -> Vec<Seen> {
        self.provider
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

#[derive(Serialize)]
struct TokenAnswer<'answer> {
    access_token: &'answer str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id_token: Option<&'answer str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<&'answer str>,
    token_type: &'static str,
}

async fn token_answer(
    State(provider): State<Provider>,
    headers: HeaderMap,
    body: String,
) -> Response {
    provider.record("POST", "/token", &headers, &body);
    let replies = provider.replies.as_ref();
    if replies.token_status != 200 {
        return StatusCode::from_u16(replies.token_status)
            .unwrap()
            .into_response();
    }
    Json(TokenAnswer {
        access_token: &replies.token.access_token,
        id_token: replies.token.id_token.as_deref(),
        refresh_token: replies.token.refresh_token.as_deref(),
        token_type: "Bearer",
    })
    .into_response()
}

async fn userinfo_answer(
    State(provider): State<Provider>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    provider.record("GET", uri.path(), &headers, "");
    let replies = provider.replies.as_ref();
    if replies.userinfo_status != 200 {
        return StatusCode::from_u16(replies.userinfo_status)
            .unwrap()
            .into_response();
    }
    Json(replies.userinfo.clone()).into_response()
}

pub(super) async fn serve(replies: Replies) -> Running {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let provider = Provider {
        replies: Arc::new(replies),
        seen: Arc::new(Mutex::new(Vec::new())),
    };
    let router = Router::new()
        .route("/token", post(token_answer))
        .route("/userinfo", get(userinfo_answer))
        .with_state(provider.clone());
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { axum::serve(listener, router).await.unwrap() });
    Running {
        base,
        provider,
        tasks,
    }
}

pub(super) fn transport() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

pub(super) struct Fixture {
    pub(super) running: Running,
    pub(super) base: String,
    pub(super) sources: SsoMappingSources,
    pub(super) pkce: PkceSettings,
    pub(super) callback_settings: GenericSsoCallbackSettings,
    pub(super) attributes: ClaimAttributes,
    pub(super) jwt_handler: JwtHandler,
    pub(super) sso_jwt_handler: Option<JwtHandler>,
    pub(super) client_id: String,
    pub(super) redirect_url: String,
}

impl Fixture {
    pub(super) async fn start(replies: Replies) -> Self {
        let running = serve(replies).await;
        let base = running.base.clone();
        let sources = mapping(&base, &[]);
        Self {
            running,
            base,
            sources,
            pkce: pkce(false),
            callback_settings: callback_settings(&[]),
            attributes: attributes(&[]),
            jwt_handler: JwtHandler::default(),
            sso_jwt_handler: None,
            client_id: CLIENT_ID.to_owned(),
            redirect_url: REDIRECT_URI.to_owned(),
        }
    }

    pub(super) fn configuration(&self) -> GenericSsoConfig<'_> {
        GenericSsoConfig {
            client_id: &self.client_id,
            redirect_url: &self.redirect_url,
            sources: &self.sources,
            pkce: &self.pkce,
            callback_settings: &self.callback_settings,
            attributes: &self.attributes,
            jwt_handler: &self.jwt_handler,
            sso_jwt_handler: self.sso_jwt_handler.as_ref(),
        }
    }

    pub(super) fn callback_url(&self, query: &str) -> String {
        format!("{}/callback?{query}", self.base)
    }

    pub(super) fn requests(&self) -> Vec<Seen> {
        self.running.requests()
    }
}

pub(super) async fn call(
    fixture: &Fixture,
    callback_url: &str,
    cookie: Option<&str>,
    rows: &FakeRows,
    cache: &FakeCache,
) -> Result<SsoExchangeResult, GenericSsoError> {
    let callback = GenericSsoCallback {
        url: callback_url,
        sso_state_cookie: cookie,
    };
    let config = fixture.configuration();
    let caches = caches(cache);
    get_generic_sso_response(&transport(), &callback, &config, rows, &caches).await
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

// the failure handler's log lines, captured on the calling thread at the given level
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
