use std::collections::{BTreeMap, VecDeque};
use std::mem;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_cache::dual_cache::DualCacheError;
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;
use tracing::Level;
use tracing::Subscriber;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::auth_cache::AuthCacheError;
use crate::jwt_keys::cache::KeyCache;
use crate::jwt_keys::clock::Clock;
use crate::jwt_keys::http::{FetchError, HttpReply, JwksHttp};
use crate::jwt_keys::resolver::JwksResolver;
use crate::jwt_keys::settings::{JwtKeySettings, SettingsError};

pub const JWKS_URL: &str = "https://idp.example.com/jwks";
pub const DISCOVERY_URL: &str = "https://idp.example.com/.well-known/openid-configuration";
pub const KEYS_KEY: &str = "litellm_jwt_auth_keys_https://idp.example.com/jwks";

pub fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub fn key(kid: &str) -> Json {
    json(&format!(
        r#"{{"kid":"{kid}","kty":"RSA","n":"material-{kid}"}}"#
    ))
}

pub fn keys(entries: &[Json]) -> Json {
    Json::List(entries.to_vec())
}

pub fn settings(url: Option<&str>, ttl: f64, stale_ttl: f64) -> JwtKeySettings {
    JwtKeySettings::new(url.map(str::to_owned), ttl, stale_ttl, None, None).unwrap()
}

pub fn settings_error(url: Option<&str>, ttl: f64, stale_ttl: f64) -> SettingsError {
    JwtKeySettings::new(url.map(str::to_owned), ttl, stale_ttl, None, None).unwrap_err()
}

pub fn cache_failure() -> AuthCacheError {
    AuthCacheError::Backend(Box::new(DualCacheError::UnrepresentableValue))
}

#[derive(Debug)]
pub struct CacheWrite {
    pub key: String,
    pub value: Json,
    pub ttl: f64,
}

#[derive(Default)]
pub struct FakeCache {
    pub entries: Mutex<BTreeMap<String, Json>>,
    pub writes: Mutex<Vec<CacheWrite>>,
    pub fail_get: Mutex<Option<String>>,
    pub fail_set: Mutex<Option<String>>,
    // the next read of this key reports a miss even though the value is stored
    pub miss_once: Mutex<Option<String>>,
}

impl FakeCache {
    pub fn entries(&self) -> BTreeMap<String, Json> {
        self.entries.lock().unwrap().clone()
    }

    pub fn writes(&self) -> Vec<CacheWrite> {
        mem::take(&mut *self.writes.lock().unwrap())
    }

    pub fn write_keys(&self) -> Vec<String> {
        self.writes
            .lock()
            .unwrap()
            .iter()
            .map(|write| write.key.clone())
            .collect()
    }

    pub fn seed(&self, key: &str, value: Json) {
        self.entries.lock().unwrap().insert(key.to_owned(), value);
    }

    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }

    pub fn remove(&self, key: &str) -> Option<Json> {
        self.entries.lock().unwrap().shift_remove(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.lock().unwrap().contains_key(key)
    }

    pub fn miss_once(&self, key: &str) {
        *self.miss_once.lock().unwrap() = Some(key.to_owned());
    }
}

pub fn stale_key() -> String {
    format!("litellm_stale_{KEYS_KEY}")
}

pub fn timestamp_key() -> String {
    format!("litellm_stale_written_at_{KEYS_KEY}")
}

pub fn unreachable_key() -> String {
    format!("litellm_jwks_unreachable_{KEYS_KEY}")
}

#[async_trait]
impl KeyCache for FakeCache {
    async fn get(&self, key: &str) -> Result<Option<Json>, AuthCacheError> {
        if self.fail_get.lock().unwrap().as_deref() == Some(key) {
            return Err(cache_failure());
        }
        let mut miss_once = self.miss_once.lock().unwrap();
        if miss_once.as_deref() == Some(key) {
            *miss_once = None;
            return Ok(None);
        }
        drop(miss_once);
        Ok(self.entries.lock().unwrap().get(key).cloned())
    }

    async fn set(&self, key: &str, value: &Json, ttl_seconds: f64) -> Result<(), AuthCacheError> {
        if self.fail_set.lock().unwrap().as_deref() == Some(key) {
            return Err(cache_failure());
        }
        self.writes.lock().unwrap().push(CacheWrite {
            key: key.to_owned(),
            value: value.clone(),
            ttl: ttl_seconds,
        });
        self.entries
            .lock()
            .unwrap()
            .insert(key.to_owned(), value.clone());
        Ok(())
    }
}

#[derive(Clone)]
pub enum Reply {
    Answer { status: u16, body: String },
    Transport,
    Failed,
}

// three transport failures in a row: one refresh that ends in an outage
pub fn outage() -> Vec<Reply> {
    vec![Reply::Transport; 3]
}

pub struct ScriptedHttp {
    replies: Mutex<VecDeque<Reply>>,
    pub requests: Mutex<Vec<String>>,
}

impl ScriptedHttp {
    pub fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }

    pub fn push(&self, reply: Reply) {
        self.replies.lock().unwrap().push_back(reply);
    }

    pub fn push_all(&self, replies: Vec<Reply>) {
        self.replies.lock().unwrap().extend(replies);
    }

    pub fn answer(body: &Json) -> Reply {
        Reply::Answer {
            status: 200,
            body: serde_json::to_string(body).unwrap(),
        }
    }

    pub fn text(status: u16, body: &str) -> Reply {
        Reply::Answer {
            status,
            body: body.to_owned(),
        }
    }
}

#[async_trait]
impl JwksHttp for ScriptedHttp {
    async fn get(&self, url: &str) -> Result<HttpReply, FetchError> {
        tokio::task::yield_now().await;
        self.requests.lock().unwrap().push(url.to_owned());
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("the scripted http ran out of replies");
        match reply {
            Reply::Answer { status, body } => Ok(HttpReply {
                status,
                body: body.into_bytes(),
            }),
            Reply::Transport => Err(FetchError::Transport(HttpError::Timeout)),
            Reply::Failed => Err(FetchError::Failed(HttpError::TooLarge { limit: 1 })),
        }
    }
}

pub struct FakeClock {
    now: Mutex<f64>,
    sleeps: Mutex<Vec<Duration>>,
}

impl FakeClock {
    pub fn new(now: f64) -> Self {
        Self {
            now: Mutex::new(now),
            sleeps: Mutex::new(Vec::new()),
        }
    }

    pub fn advance(&self, seconds: f64) {
        *self.now.lock().unwrap() += seconds;
    }

    pub fn sleeps(&self) -> Vec<Duration> {
        self.sleeps.lock().unwrap().clone()
    }
}

#[async_trait]
impl Clock for FakeClock {
    fn now(&self) -> f64 {
        *self.now.lock().unwrap()
    }

    async fn sleep(&self, delay: Duration) {
        self.sleeps.lock().unwrap().push(delay);
    }
}

pub struct Fixture {
    pub resolver: JwksResolver,
    pub cache: Arc<FakeCache>,
    pub http: Arc<ScriptedHttp>,
    pub clock: Arc<FakeClock>,
}

pub fn resolver_over(
    settings: JwtKeySettings,
    cache: Arc<FakeCache>,
    http: Arc<ScriptedHttp>,
    clock: Arc<FakeClock>,
) -> JwksResolver {
    let cache_tier: Arc<dyn KeyCache> = Arc::clone(&cache);
    let http_source: Arc<dyn JwksHttp> = Arc::clone(&http);
    let wall_clock: Arc<dyn Clock> = Arc::clone(&clock);
    JwksResolver::new(Arc::new(settings), cache_tier, http_source, wall_clock)
}

pub fn fixture(settings: JwtKeySettings, clock_now: f64, replies: Vec<Reply>) -> Fixture {
    let cache = Arc::new(FakeCache::default());
    let http = Arc::new(ScriptedHttp::new(replies));
    let clock = Arc::new(FakeClock::new(clock_now));
    let resolver = resolver_over(
        settings,
        Arc::clone(&cache),
        Arc::clone(&http),
        Arc::clone(&clock),
    );
    Fixture {
        resolver,
        cache,
        http,
        clock,
    }
}

pub struct LevelCapture {
    levels: Arc<Mutex<Vec<Level>>>,
    messages: Arc<Mutex<Vec<String>>>,
    _guard: DefaultGuard,
}

impl LevelCapture {
    pub fn new() -> Self {
        let levels = Arc::new(Mutex::new(Vec::new()));
        let messages = Arc::new(Mutex::new(Vec::new()));
        let subscriber = registry().with(RecordLevels {
            levels: Arc::clone(&levels),
            messages: Arc::clone(&messages),
        });
        Self {
            levels,
            messages,
            _guard: set_default(subscriber),
        }
    }

    pub fn levels(&self) -> Vec<Level> {
        self.levels.lock().unwrap().clone()
    }

    pub fn messages(&self) -> Vec<String> {
        self.messages.lock().unwrap().clone()
    }
}

struct RecordLevels {
    levels: Arc<Mutex<Vec<Level>>>,
    messages: Arc<Mutex<Vec<String>>>,
}

impl<S: Subscriber> Layer<S> for RecordLevels {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        self.levels.lock().unwrap().push(*event.metadata().level());
        let mut visitor = MessageField(String::new());
        event.record(&mut visitor);
        self.messages.lock().unwrap().push(visitor.0);
    }
}

struct MessageField(String);

impl tracing::field::Visit for MessageField {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}
