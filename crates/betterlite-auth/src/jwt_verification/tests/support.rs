use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_cache::dual_cache::DualCacheError;
use betterlite_config::sections::jwt_settings::Settings as JwtAuthSettings;
use betterlite_config::sections::jwt_settings::groups::{
    ClaimFields, KeyFields, RbacFields, RouteFields, RoutingFields, ScopeFields, SyncFields,
    TeamFields, UserinfoFields, VirtualKeyFields,
};
use betterlite_config::sections::jwt_settings::records::{IssuerConfig, Selector};
use betterlite_models::decisions::Json;
use betterlite_paths::paths::Document;
use indexmap::IndexMap;
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::auth_cache::AuthCacheError;
use crate::jwt_keys::cache::KeyCache;
use crate::jwt_keys::clock::Clock;
use crate::jwt_keys::errors::KeyError;
use crate::jwt_keys::http::FetchError;
use crate::jwt_verification::settings::JwtVerificationSettings;
use crate::jwt_verification::source::PublicKeySource;
use crate::jwt_verification::userinfo::{UserinfoHttp, UserinfoReply};
use crate::jwt_verification::verifier::JwtVerifier;

use super::fixtures;

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(super) fn parse_json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn document(text: &str) -> Document {
    Document {
        json: serde_json::from_str(text).unwrap(),
    }
}

pub(super) fn claim_fields(payload: &Document) -> IndexMap<String, Json> {
    serde_json::from_value(payload.json.clone()).unwrap()
}

pub(super) fn text_claim<'fields>(
    fields: &'fields IndexMap<String, Json>,
    name: &str,
) -> Option<&'fields str> {
    match fields.get(name) {
        Some(Json::Text(text)) => Some(text.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

#[derive(Clone, Default)]
pub(super) struct Events {
    lines: Arc<Mutex<Vec<String>>>,
}

impl Events {
    pub(super) fn push(&self, event: String) {
        lock(&self.lines).push(event);
    }

    pub(super) fn take(&self) -> Vec<String> {
        std::mem::take(&mut *lock(&self.lines))
    }
}

pub(super) struct TestClock {
    pub(super) now: Mutex<f64>,
}

impl Default for TestClock {
    fn default() -> Self {
        Self {
            now: Mutex::new(fixtures::NOW),
        }
    }
}

#[async_trait]
impl Clock for TestClock {
    fn now(&self) -> f64 {
        *lock(&self.now)
    }

    async fn sleep(&self, _delay: Duration) {}
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct WriteRecord {
    pub(super) key: String,
    pub(super) ttl: f64,
    pub(super) value: Json,
}

#[derive(Default)]
pub(super) struct TestCache {
    pub(super) entries: Mutex<BTreeMap<String, Json>>,
    pub(super) writes: Mutex<Vec<WriteRecord>>,
    pub(super) fail_read: AtomicBool,
    pub(super) fail_write: AtomicBool,
    pub(super) events: Events,
}

impl TestCache {
    pub(super) fn with_entry(key: &str, value: Json) -> Self {
        let cache = Self::default();
        lock(&cache.entries).insert(key.to_owned(), value);
        cache
    }

    pub(super) fn failing_read() -> Self {
        let cache = Self::default();
        cache.fail_read.store(true, Ordering::Release);
        cache
    }

    pub(super) fn failing_write() -> Self {
        let cache = Self::default();
        cache.fail_write.store(true, Ordering::Release);
        cache
    }
}

fn cache_error() -> AuthCacheError {
    AuthCacheError::from(DualCacheError::UnrepresentableValue)
}

#[async_trait]
impl KeyCache for TestCache {
    async fn read_value(&self, key: &str) -> Result<Option<Json>, AuthCacheError> {
        self.events.push(format!("read:{key}"));
        if self.fail_read.load(Ordering::Acquire) {
            return Err(cache_error());
        }
        Ok(lock(&self.entries).get(key).cloned())
    }

    async fn write_value(
        &self,
        key: &str,
        value: &Json,
        ttl_seconds: f64,
    ) -> Result<(), AuthCacheError> {
        self.events.push(format!("write:{key}"));
        if self.fail_write.load(Ordering::Acquire) {
            return Err(cache_error());
        }
        lock(&self.entries).insert(key.to_owned(), value.clone());
        lock(&self.writes).push(WriteRecord {
            key: key.to_owned(),
            ttl: ttl_seconds,
            value: value.clone(),
        });
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(super) enum FakeReply {
    Answer { status: u16, body: String },
    Transport,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HttpRequestRecord {
    pub(super) url: String,
    pub(super) authorization: String,
    pub(super) accept: String,
}

#[derive(Default)]
pub(super) struct TestHttp {
    pub(super) replies: Mutex<VecDeque<FakeReply>>,
    pub(super) requests: Mutex<Vec<HttpRequestRecord>>,
    pub(super) closed: AtomicBool,
    pub(super) events: Events,
}

impl TestHttp {
    pub(super) fn with_replies(replies: Vec<FakeReply>) -> Self {
        let http = Self::default();
        lock(&http.replies).extend(replies);
        http
    }

    pub(super) fn requests(&self) -> Vec<HttpRequestRecord> {
        lock(&self.requests).clone()
    }
}

#[async_trait]
impl UserinfoHttp for TestHttp {
    async fn get(
        &self,
        url: &str,
        authorization: &str,
        accept: &str,
    ) -> Result<UserinfoReply, FetchError> {
        self.events.push(format!("http:{url}"));
        lock(&self.requests).push(HttpRequestRecord {
            url: url.to_owned(),
            authorization: authorization.to_owned(),
            accept: accept.to_owned(),
        });
        match lock(&self.replies).pop_front() {
            Some(FakeReply::Answer { status, body }) => Ok(UserinfoReply {
                status,
                body: body.into_bytes(),
            }),
            Some(FakeReply::Transport) | None => Err(FetchError::Transport(
                betterlite_http::error::HttpError::Timeout,
            )),
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
}

#[derive(Clone, Debug)]
pub(super) enum SourceReply {
    Key(String),
    Null,
    Unreachable,
    NoMatchingKey,
}

pub(super) struct ScriptedSource {
    pub(super) global: Mutex<VecDeque<SourceReply>>,
    pub(super) by_url: Mutex<BTreeMap<String, SourceReply>>,
    pub(super) calls: Mutex<Vec<String>>,
}

impl ScriptedSource {
    pub(super) fn with_keys(key: &str) -> Self {
        let source = Self::default();
        lock(&source.global).push_back(SourceReply::Key(key.to_owned()));
        source
    }

    pub(super) fn with_global(replies: Vec<SourceReply>) -> Self {
        let source = Self::default();
        lock(&source.global).extend(replies);
        source
    }

    pub(super) fn with_url(url: &str, reply: SourceReply) -> Self {
        let source = Self::default();
        lock(&source.by_url).insert(url.to_owned(), reply);
        source
    }

    pub(super) fn calls(&self) -> Vec<String> {
        lock(&self.calls).clone()
    }
}

impl Default for ScriptedSource {
    fn default() -> Self {
        Self {
            global: Mutex::new(VecDeque::new()),
            by_url: Mutex::new(BTreeMap::new()),
            calls: Mutex::new(Vec::new()),
        }
    }
}

fn source_reply(reply: SourceReply, kid: Option<&str>) -> Result<Json, KeyError> {
    match reply {
        SourceReply::Key(text) => Ok(parse_json(&text)),
        SourceReply::Null => Ok(Json::Null),
        SourceReply::Unreachable => Err(KeyError::Unreachable {
            message: "the connection timed out".to_owned(),
        }),
        SourceReply::NoMatchingKey => Err(KeyError::NoMatchingKey {
            keys: "https://idp.example.com/jwks".to_owned(),
            kid: kid.map(str::to_owned),
        }),
    }
}

#[async_trait]
impl PublicKeySource for ScriptedSource {
    async fn resolve_public_key(&self, kid: Option<&str>) -> Result<Json, KeyError> {
        lock(&self.calls).push(format!("lookup:{}", kid.unwrap_or("None")));
        let reply = lock(&self.global)
            .pop_front()
            .unwrap_or(SourceReply::Unreachable);
        source_reply(reply, kid)
    }

    async fn resolve_keys_for_url(
        &self,
        jwks_url: &str,
        kid: Option<&str>,
    ) -> Result<Json, KeyError> {
        lock(&self.calls).push(format!("issuer:{jwks_url}"));
        let reply = lock(&self.by_url)
            .get(jwks_url)
            .cloned()
            .unwrap_or(SourceReply::NoMatchingKey);
        source_reply(reply, kid)
    }
}

#[derive(Clone, Default)]
pub(super) struct CapturedLogs {
    pub(super) lines: Arc<Mutex<Vec<String>>>,
}

impl CapturedLogs {
    pub(super) fn take(&self) -> Vec<String> {
        std::mem::take(&mut *lock(&self.lines))
    }

    pub(super) fn warnings(&self) -> usize {
        lock(&self.lines)
            .iter()
            .filter(|line| line.starts_with("WARN"))
            .count()
    }
}

#[derive(Default)]
struct MessageText {
    text: Option<String>,
}

impl Visit for MessageText {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.text = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.text = Some(format!("{value:?}"));
        }
    }
}

pub(super) struct CaptureLayer {
    pub(super) logs: CapturedLogs,
}

impl<S: Subscriber> Layer<S> for CaptureLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let mut message = MessageText::default();
        event.record(&mut message);
        let metadata = event.metadata();
        let line = format!(
            "{} {} {}",
            metadata.level(),
            metadata.target(),
            message.text.unwrap_or_default()
        );
        lock(&self.logs.lines).push(line);
    }
}

// Captures DEBUG through ERROR for the calling thread, which is the thread a current_thread runtime runs on.
pub(super) fn capture_logs() -> (CapturedLogs, DefaultGuard) {
    let logs = CapturedLogs::default();
    let subscriber = registry().with(CaptureLayer { logs: logs.clone() });
    let guard = tracing::subscriber::set_default(subscriber);
    (logs, guard)
}

pub(super) struct Harness {
    pub(super) verifier: JwtVerifier,
    pub(super) cache: Arc<TestCache>,
    pub(super) http: Arc<TestHttp>,
}

pub(super) fn settings(
    audience: Option<&str>,
    issuer: Option<&str>,
    jwt: JwtAuthSettings,
) -> JwtVerificationSettings {
    JwtVerificationSettings {
        audience: audience.map(str::to_owned),
        issuer: issuer.map(str::to_owned),
        jwt,
    }
}

pub(super) fn jwt_settings_with_issuers(issuers: Vec<IssuerConfig>) -> JwtAuthSettings {
    jwt_auth_settings(
        RoutingFields {
            routing_overrides: None,
            team_claim_fallback: false,
            fallback_to_db_teams: false,
            issuers: Some(issuers),
        },
        UserinfoFields::default(),
    )
}

pub(super) fn jwt_settings_with_userinfo(endpoint: Option<&str>, ttl: f64) -> JwtAuthSettings {
    jwt_auth_settings(
        RoutingFields::default(),
        UserinfoFields {
            oidc_userinfo_endpoint: endpoint.map(str::to_owned),
            oidc_userinfo_enabled: false,
            oidc_userinfo_cache_ttl: ttl,
        },
    )
}

// every litellm_jwtauth group named, so a default this helper does not set stays visible at the call site
fn jwt_auth_settings(routing: RoutingFields, userinfo: UserinfoFields) -> JwtAuthSettings {
    JwtAuthSettings {
        routes: RouteFields::default(),
        teams: TeamFields::default(),
        claims: ClaimFields::default(),
        keys: KeyFields::default(),
        rbac: RbacFields::default(),
        scopes: ScopeFields::default(),
        custom_validate: None,
        sync: SyncFields::default(),
        userinfo,
        virtual_keys: VirtualKeyFields::default(),
        routing,
    }
}

pub(super) fn issuer_config(issuer: &str) -> IssuerConfig {
    IssuerConfig {
        issuer: issuer.to_owned(),
        jwks_url: None,
        audience: None,
        disable_audience_validation: false,
        user_id_jwt_field: None,
        user_email_jwt_field: None,
        team_id_jwt_field: None,
        team_ids_jwt_field: None,
        org_id_jwt_field: None,
        end_user_id_jwt_field: None,
        virtual_key_claim_field: None,
        unregistered_client: None,
    }
}

pub(super) fn audience_one(value: &str) -> Option<Selector> {
    Some(Selector::One(value.to_owned()))
}

pub(super) fn harness(settings: JwtVerificationSettings) -> Harness {
    harness_with(settings, TestCache::default(), TestHttp::default())
}

pub(super) fn harness_with(
    settings: JwtVerificationSettings,
    cache: TestCache,
    http: TestHttp,
) -> Harness {
    let cache = Arc::new(cache);
    let http = Arc::new(http);
    let clock = Arc::new(TestClock::default());
    let verifier = JwtVerifier::new(
        Arc::new(settings),
        Arc::clone(&cache),
        Arc::clone(&http),
        clock,
    );
    Harness {
        verifier,
        cache,
        http,
    }
}
