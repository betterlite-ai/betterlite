use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::mem;
use std::ops::AddAssign as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task;

use async_trait::async_trait;
use betterlite_cache::dual_cache::DualCacheError;
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;
use tracing::Level;
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::auth_cache::AuthCacheError;
use crate::jwt_keys::cache::KeyCache;
use crate::jwt_keys::clock::Clock;
use crate::jwt_keys::http::{FetchError, HttpReply, JwksHttp};
use crate::jwt_keys::resolver::JwksResolver;
use crate::jwt_keys::settings::{JwtKeySettings, SettingsError};

pub(super) const JWKS_URL: &str = "https://idp.example.com/jwks";
pub(super) const DISCOVERY_URL: &str = "https://idp.example.com/.well-known/openid-configuration";
pub(super) const KEYS_KEY: &str = "litellm_jwt_auth_keys_https://idp.example.com/jwks";

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn key(kid: &str) -> Json {
    json(&format!(
        r#"{{"kid":"{kid}","kty":"RSA","n":"material-{kid}"}}"#
    ))
}

pub(super) fn keys(entries: &[Json]) -> Json {
    Json::List(entries.to_vec())
}

pub(super) fn settings(url: Option<&str>, ttl: f64, stale_ttl: f64) -> JwtKeySettings {
    JwtKeySettings::new(url.map(str::to_owned), ttl, stale_ttl, None, None).unwrap()
}

pub(super) fn settings_error(url: Option<&str>, ttl: f64, stale_ttl: f64) -> SettingsError {
    JwtKeySettings::new(url.map(str::to_owned), ttl, stale_ttl, None, None).unwrap_err()
}

pub(super) fn cache_failure() -> AuthCacheError {
    AuthCacheError::Backend(Box::new(DualCacheError::UnrepresentableValue))
}

#[derive(Debug)]
pub(super) struct CacheWrite {
    pub(super) key: String,
    pub(super) value: Json,
    pub(super) ttl: f64,
}

#[derive(Default)]
pub(super) struct FakeCache {
    pub(super) entries: Mutex<BTreeMap<String, Json>>,
    pub(super) writes: Mutex<Vec<CacheWrite>>,
    pub(super) fail_get: Mutex<Option<String>>,
    pub(super) fail_set: Mutex<Option<String>>,
    // the next read of this key reports a miss even though the value is stored
    pub(super) miss_once: Mutex<Option<String>>,
}

impl FakeCache {
    pub(super) fn writes(&self) -> Vec<CacheWrite> {
        mem::take(&mut *self.writes.lock().unwrap())
    }

    pub(super) fn write_keys(&self) -> Vec<String> {
        self.writes
            .lock()
            .unwrap()
            .iter()
            .map(|write| write.key.clone())
            .collect()
    }

    pub(super) fn seed(&self, key: &str, value: Json) {
        self.entries.lock().unwrap().insert(key.to_owned(), value);
    }

    pub(super) fn remove(&self, key: &str) -> Option<Json> {
        self.entries.lock().unwrap().remove(key)
    }

    pub(super) fn contains(&self, key: &str) -> bool {
        self.entries.lock().unwrap().contains_key(key)
    }

    pub(super) fn miss_once(&self, key: &str) {
        *self.miss_once.lock().unwrap() = Some(key.to_owned());
    }
}

pub(super) fn stale_key() -> String {
    format!("litellm_stale_{KEYS_KEY}")
}

pub(super) fn timestamp_key() -> String {
    format!("litellm_stale_written_at_{KEYS_KEY}")
}

pub(super) fn unreachable_key() -> String {
    format!("litellm_jwks_unreachable_{KEYS_KEY}")
}

#[async_trait]
impl KeyCache for FakeCache {
    async fn read_value(&self, key: &str) -> Result<Option<Json>, AuthCacheError> {
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

    async fn write_value(
        &self,
        key: &str,
        value: &Json,
        ttl_seconds: f64,
    ) -> Result<(), AuthCacheError> {
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
pub(super) enum Reply {
    Answer { status: u16, body: String },
    Transport,
    Failed,
}

// three transport failures in a row: one refresh that ends in an outage
pub(super) fn outage() -> Vec<Reply> {
    vec![Reply::Transport; 3]
}

pub(super) struct ScriptedHttp {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<String>>,
}

impl ScriptedHttp {
    pub(super) fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }

    pub(super) fn push(&self, reply: Reply) {
        self.replies.lock().unwrap().push_back(reply);
    }

    pub(super) fn push_all(&self, replies: Vec<Reply>) {
        self.replies.lock().unwrap().extend(replies);
    }

    pub(super) fn answer(body: &Json) -> Reply {
        Reply::Answer {
            status: 200,
            body: serde_json::to_string(body).unwrap(),
        }
    }

    pub(super) fn text(status: u16, body: &str) -> Reply {
        Reply::Answer {
            status,
            body: body.to_owned(),
        }
    }
}

#[async_trait]
impl JwksHttp for ScriptedHttp {
    async fn get(&self, url: &str) -> Result<HttpReply, FetchError> {
        task::yield_now().await;
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

pub(super) struct FakeClock {
    now: Mutex<f64>,
    sleeps: Mutex<Vec<Duration>>,
}

impl FakeClock {
    pub(super) fn new(now: f64) -> Self {
        Self {
            now: Mutex::new(now),
            sleeps: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn advance(&self, seconds: f64) {
        self.now.lock().unwrap().add_assign(seconds);
    }

    pub(super) fn sleeps(&self) -> Vec<Duration> {
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

pub(super) struct ResolverFixture {
    pub(super) resolver: JwksResolver,
    pub(super) cache: Arc<FakeCache>,
    pub(super) http: Arc<ScriptedHttp>,
    pub(super) clock: Arc<FakeClock>,
}

pub(super) fn resolver_over(
    settings: JwtKeySettings,
    cache: &Arc<FakeCache>,
    http: &Arc<ScriptedHttp>,
    clock: &Arc<FakeClock>,
) -> JwksResolver {
    let cache_tier: Arc<dyn KeyCache> = Arc::<FakeCache>::clone(cache);
    let http_source: Arc<dyn JwksHttp> = Arc::<ScriptedHttp>::clone(http);
    let wall_clock: Arc<dyn Clock> = Arc::<FakeClock>::clone(clock);
    JwksResolver::new(Arc::new(settings), cache_tier, http_source, wall_clock)
}

pub(super) fn new_resolver_fixture(
    settings: JwtKeySettings,
    clock_now: f64,
    replies: Vec<Reply>,
) -> ResolverFixture {
    let cache = Arc::new(FakeCache::default());
    let http = Arc::new(ScriptedHttp::new(replies));
    let clock = Arc::new(FakeClock::new(clock_now));
    let resolver = resolver_over(settings, &cache, &http, &clock);
    ResolverFixture {
        resolver,
        cache,
        http,
        clock,
    }
}

pub(super) struct LevelCapture {
    levels: Arc<Mutex<Vec<Level>>>,
    messages: Arc<Mutex<Vec<String>>>,
    _guard: DefaultGuard,
}

impl LevelCapture {
    pub(super) fn new() -> Self {
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

    pub(super) fn levels(&self) -> Vec<Level> {
        self.levels.lock().unwrap().clone()
    }

    pub(super) fn messages(&self) -> Vec<String> {
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

impl Visit for MessageField {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}
