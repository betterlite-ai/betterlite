use std::collections::VecDeque;
use std::fmt::Debug;
use std::future::{Future, pending, ready};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::backend::{BackendError, CacheResult, Ttl};
use betterlite_cache::dual_cache::{
    DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE, MemoryMaxSize,
};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_redis::redis_client_reuse::error::ClientReuseError;
use betterlite_redis::redis_live_client::LiveClientError;
use redis::RedisError;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::Notify;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::auth_cache::{AuthCache, gateway_cache};
use crate::auth_pubsub::channel::invalidation_channel;
use crate::auth_pubsub::clock::BackoffClock;
use crate::auth_pubsub::endpoint::{
    AuthPubSub, AuthPubSubSubscription, PubSubMessage, TransportError,
};
use crate::auth_pubsub::eviction::{CacheError, LocalEviction};
use crate::auth_pubsub::message::Invalidation;

pub(crate) const CLOCK_NOW: f64 = 1000.0_f64;
pub(crate) const HEX_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn scripted_failure(text: &str) -> ClientReuseError {
    ClientReuseError::Live(LiveClientError::Redis(RedisError::from(io::Error::other(
        text.to_owned(),
    ))))
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Entry {
    Subscribe,
    SubscribeFailed,
    Poll(Duration),
    Message,
    Close,
    CloseFailed,
    Publish(String),
    PublishFailed,
    Evict(String),
    EvictFailed,
    Sleep(Duration),
    PublishStarted,
}

pub(crate) struct Probe {
    events: Mutex<Vec<Entry>>,
    in_flight: AtomicUsize,
    peak_in_flight: AtomicUsize,
    started: AtomicUsize,
    blocked_dropped: AtomicUsize,
}

impl Probe {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::new(Vec::new()),
            in_flight: AtomicUsize::new(0),
            peak_in_flight: AtomicUsize::new(0),
            started: AtomicUsize::new(0),
            blocked_dropped: AtomicUsize::new(0),
        })
    }

    pub(crate) fn record(&self, event: Entry) {
        lock(&self.events).push(event);
    }

    pub(crate) fn events(&self) -> Vec<Entry> {
        lock(&self.events).clone()
    }

    pub(crate) fn count(&self, wanted: &Entry) -> usize {
        self.events()
            .iter()
            .filter(|event| **event == *wanted)
            .count()
    }

    pub(crate) fn publishes(&self) -> usize {
        self.events()
            .iter()
            .filter(|entry| matches!(entry, Entry::Publish(_)))
            .count()
    }

    pub(crate) fn started(&self) -> usize {
        self.started.load(Ordering::SeqCst)
    }

    pub(crate) fn blocked_dropped(&self) -> usize {
        self.blocked_dropped.load(Ordering::SeqCst)
    }

    pub(crate) fn peak_in_flight(&self) -> usize {
        self.peak_in_flight.load(Ordering::SeqCst)
    }

    fn enter_publish(&self) {
        let current = self
            .in_flight
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        self.peak_in_flight.fetch_max(current, Ordering::SeqCst);
        _ = self.started.fetch_add(1, Ordering::SeqCst);
    }

    fn leave_publish(&self) {
        _ = self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Clone)]
pub(crate) enum PublishMode {
    Accept,
    Fail,
    Block(Arc<Notify>),
}

#[derive(Clone, Copy)]
pub(crate) enum CloseMode {
    Accept,
    Fail,
}

#[derive(Clone)]
pub(crate) enum Step {
    Message(Vec<u8>),
    Empty,
    Fail,
    Block,
}

#[derive(Clone)]
pub(crate) enum Session {
    Failure,
    Serving(Vec<Step>),
}

pub(crate) struct ScriptedEndpoint {
    pub(crate) probe: Arc<Probe>,
    sessions: Mutex<VecDeque<Session>>,
    mode: PublishMode,
    close: CloseMode,
}

impl ScriptedEndpoint {
    pub(crate) fn new(probe: Arc<Probe>, sessions: Vec<Session>, mode: PublishMode) -> Self {
        Self {
            probe,
            sessions: Mutex::new(VecDeque::from(sessions)),
            mode,
            close: CloseMode::Accept,
        }
    }

    pub(crate) fn with_close(mut self, close: CloseMode) -> Self {
        self.close = close;
        self
    }
}

fn resolved<T>(
    probe: &Probe,
    event: Entry,
    outcome: Result<T, TransportError>,
) -> impl Future<Output = Result<T, TransportError>> + Send
where
    T: Send,
{
    probe.record(event);
    ready(outcome)
}

impl AuthPubSub for ScriptedEndpoint {
    type Subscription = ScriptedSubscription;

    fn channel(&self) -> String {
        invalidation_channel(None)
    }

    async fn publish(&self, message: &str) -> Result<(), TransportError> {
        self.probe.enter_publish();
        self.probe.record(Entry::PublishStarted);
        match &self.mode {
            PublishMode::Accept => {
                self.probe.record(Entry::Publish(message.to_owned()));
                self.probe.leave_publish();
                Ok(())
            }
            PublishMode::Fail => {
                self.probe.record(Entry::PublishFailed);
                self.probe.leave_publish();
                Err(TransportError::Publish(scripted_failure(
                    "the scripted publish failed",
                )))
            }
            PublishMode::Block(release) => {
                let guard = Dropped(Arc::clone(&self.probe));
                release.notified().await;
                drop(guard);
                self.probe.record(Entry::Publish(message.to_owned()));
                self.probe.leave_publish();
                Ok(())
            }
        }
    }

    fn subscribe(&self) -> impl Future<Output = Result<Self::Subscription, TransportError>> + Send {
        let session = lock(&self.sessions).pop_front();
        let close = self.close;
        let probe = Arc::clone(&self.probe);
        let outcome = match session {
            None => Err(TransportError::Connection(scripted_failure(
                "the scripted sessions ran out",
            ))),
            Some(Session::Failure) => {
                probe.record(Entry::SubscribeFailed);
                Err(TransportError::Connection(scripted_failure(
                    "the scripted subscribe failed",
                )))
            }
            Some(Session::Serving(steps)) => Ok(ScriptedSubscription {
                probe,
                steps: Mutex::new(VecDeque::from(steps)),
                close,
            }),
        };
        resolved(&self.probe, Entry::Subscribe, outcome)
    }
}

struct Dropped(Arc<Probe>);

impl Drop for Dropped {
    fn drop(&mut self) {
        _ = self.0.blocked_dropped.fetch_add(1, Ordering::SeqCst);
    }
}

pub(crate) struct ScriptedSubscription {
    probe: Arc<Probe>,
    steps: Mutex<VecDeque<Step>>,
    close: CloseMode,
}

impl AuthPubSubSubscription for ScriptedSubscription {
    async fn next_message(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<PubSubMessage>, TransportError> {
        self.probe.record(Entry::Poll(timeout));
        let step = lock(&self.steps).pop_front();
        match step {
            Some(Step::Message(data)) => {
                self.probe.record(Entry::Message);
                Ok(Some(PubSubMessage { data: Some(data) }))
            }
            Some(Step::Empty) => Ok(None),
            Some(Step::Fail) => Err(TransportError::Connection(scripted_failure(
                "the scripted consume failed",
            ))),
            Some(Step::Block) | None => {
                pending::<Result<Option<PubSubMessage>, TransportError>>().await
            }
        }
    }

    fn close(&mut self) -> impl Future<Output = Result<(), TransportError>> + Send {
        let failure = match self.close {
            CloseMode::Accept => None,
            CloseMode::Fail => {
                self.probe.record(Entry::CloseFailed);
                Some(TransportError::Connection(scripted_failure(
                    "the scripted close failed",
                )))
            }
        };
        resolved(
            &self.probe,
            Entry::Close,
            failure.map_or(Ok::<(), TransportError>(()), Err),
        )
    }
}

pub(crate) struct ScriptedClock {
    probe: Arc<Probe>,
}

impl ScriptedClock {
    pub(crate) fn new(probe: Arc<Probe>) -> Self {
        Self { probe }
    }
}

impl BackoffClock for ScriptedClock {
    async fn sleep(&self, duration: Duration) {
        self.probe.record(Entry::Sleep(duration));
    }
}

pub(crate) struct ScriptedEviction {
    probe: Arc<Probe>,
    failures: Mutex<Vec<String>>,
}

impl ScriptedEviction {
    pub(crate) fn new(probe: Arc<Probe>, failures: Vec<String>) -> Self {
        Self {
            probe,
            failures: Mutex::new(failures),
        }
    }
}

impl LocalEviction for ScriptedEviction {
    fn apply(&self, invalidation: Invalidation) -> Result<(), CacheError> {
        self.probe
            .record(Entry::Evict(invalidation.cache_key.clone()));
        let failing = lock(&self.failures).contains(&invalidation.cache_key);
        if failing {
            self.probe.record(Entry::EvictFailed);
            return Err(CacheError::Eviction {
                cache_key: invalidation.cache_key,
                source: BackendError::Unsupported("scripted eviction failure"),
            });
        }
        Ok(())
    }
}

fn fixed_clock() -> Arc<dyn Fn() -> f64 + Send + Sync> {
    Arc::new(|| CLOCK_NOW)
}

pub(crate) fn memory() -> Arc<LocalMemoryOps> {
    Arc::new(LocalMemoryOps::new(1024, fixed_clock()).expect("the local memory tier builds"))
}

fn settings() -> DualCacheSettings {
    DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    }
}

pub(crate) type LocalCache = AuthCache<DualCache<FaultingMemory>>;

pub(crate) struct FaultingMemory {
    inner: Arc<LocalMemoryOps>,
    fail_set: AtomicBool,
    fail_delete: AtomicBool,
}

impl FaultingMemory {
    pub(crate) fn new(inner: Arc<LocalMemoryOps>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            fail_set: AtomicBool::new(false),
            fail_delete: AtomicBool::new(false),
        })
    }
}

impl MemoryCacheOps for FaultingMemory {
    fn default_ttl(&self) -> i64 {
        self.inner.default_ttl()
    }

    fn get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        self.inner.get_cache(key)
    }

    fn set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        if self.fail_set.load(Ordering::SeqCst) {
            return Err(BackendError::Unsupported("scripted set failure"));
        }
        self.inner.set_cache(key, value, ttl)
    }

    fn delete_cache(&self, key: &str) -> CacheResult<()> {
        if self.fail_delete.load(Ordering::SeqCst) {
            return Err(BackendError::Unsupported("scripted delete failure"));
        }
        self.inner.delete_cache(key)
    }

    fn flush_cache(&self) -> CacheResult<()> {
        self.inner.flush_cache()
    }

    fn batch_get_cache(&self, keys: &[String]) -> CacheResult<Vec<Option<CacheValue>>> {
        self.inner.batch_get_cache(keys)
    }

    fn increment_cache(&self, key: &str, value: f64, ttl: Ttl) -> CacheResult<f64> {
        self.inner.increment_cache(key, value, ttl)
    }

    fn ttl_entries(&self) -> CacheResult<Vec<(String, f64)>> {
        self.inner.ttl_entries()
    }
}

impl MemoryMaxSize for FaultingMemory {
    fn set_max_size(&self, max_size: usize) {
        self.inner.set_max_size(max_size);
    }
}

pub(crate) fn store<M>(memory: &M, key: &str, value: f64)
where
    M: MemoryCacheOps,
{
    memory
        .set_cache(key, CacheValue::Float(value), None)
        .expect("the local memory tier accepts the seeded value");
}

pub(crate) fn stored(memory: &FaultingMemory, key: &str) -> Option<f64> {
    let value = memory
        .get_cache(key)
        .expect("the local memory tier answers")?;
    let CacheValue::Float(number) = value else {
        return None;
    };
    Some(number)
}

pub(crate) fn expiry(memory: &FaultingMemory, key: &str) -> Option<f64> {
    memory
        .ttl_entries()
        .expect("the local memory tier lists expiries")
        .into_iter()
        .find_map(|(name, at)| (name == key).then_some(at))
}

pub(crate) struct Fixture {
    pub(crate) cache: Arc<LocalCache>,
    pub(crate) ordinary: Arc<FaultingMemory>,
    pub(crate) key_objects: Arc<FaultingMemory>,
}

pub(crate) fn fixture() -> Fixture {
    let ordinary = FaultingMemory::new(memory());
    let key_objects = FaultingMemory::new(memory());
    let cache = gateway_cache(
        Arc::clone(&ordinary),
        Arc::clone(&key_objects),
        None,
        settings(),
        45.0_f64,
    );
    Fixture {
        cache: Arc::new(cache),
        ordinary,
        key_objects,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CapturedEvent {
    pub(crate) level: String,
    pub(crate) subject: Option<String>,
    pub(crate) backoff_seconds: Option<f64>,
}

#[derive(Default)]
struct CapturedFields {
    subject: Option<String>,
    backoff_seconds: Option<f64>,
}

struct FieldVisitor<'fields>(&'fields mut CapturedFields);

impl Visit for FieldVisitor<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "subject" {
            self.0.subject = Some(value.to_owned());
        }
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        if field.name() == "backoff_seconds" {
            self.0.backoff_seconds = Some(value);
        }
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn Debug) {}
}

pub(crate) struct CaptureLayer(Arc<Mutex<Vec<CapturedEvent>>>);

impl<Inner: Subscriber> Layer<Inner> for CaptureLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if !event
            .metadata()
            .target()
            .starts_with("betterlite_auth::auth_pubsub")
        {
            return;
        }
        let mut fields = CapturedFields::default();
        event.record(&mut FieldVisitor(&mut fields));
        let level = match *event.metadata().level() {
            Level::WARN => "warn".to_owned(),
            Level::DEBUG => "debug".to_owned(),
            Level::ERROR => "error".to_owned(),
            Level::INFO => "info".to_owned(),
            Level::TRACE => "trace".to_owned(),
        };
        lock(&self.0).push(CapturedEvent {
            level,
            subject: fields.subject,
            backoff_seconds: fields.backoff_seconds,
        });
    }
}

pub(crate) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds")
}

pub(crate) fn capture<T, F>(operation: F) -> (T, Vec<CapturedEvent>)
where
    F: FnOnce() -> T,
{
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = registry().with(CaptureLayer(Arc::clone(&events)));
    let output = with_default(subscriber, operation);
    let captured = lock(&events).clone();
    (output, captured)
}
