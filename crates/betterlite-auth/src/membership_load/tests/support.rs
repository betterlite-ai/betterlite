#![cfg(test)]

use std::collections::BTreeMap;
use std::future::pending;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, LookupClock};
use betterlite_db::database_faults::{DbError as FaultDbError, ErrorType};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::team_rows::membership::{Counter, LiteLLMTeamMembership};
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use tokio::sync::Semaphore;
use tokio::task::{Id, yield_now};
use tokio::time::timeout;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::auth_cache::keys::CacheKey;
use crate::auth_cache::payload::{self, PayloadError};
use crate::auth_cache::{AuthCacheError, gateway_cache};
use crate::auth_invalidation::AuthInvalidation;
use crate::auth_invalidation::clock::SystemClock;
use crate::auth_invalidation::ports::{AuthInvalidationCache, SpendCounterCache};
use crate::auth_invalidation::registry::{InflightLoad, MembershipInflight};
use crate::auth_invalidation::transport::InvalidationTransport;
use crate::auth_pubsub::eviction::LocalEviction;
use crate::auth_read::table::{
    AuthReadError, AuthTable, Create, FindMany, FindOne, ReadContext, Update,
};
use crate::membership_load::MembershipLookup;
use crate::membership_load::cache::MembershipCache;
use crate::membership_load::error::{MembershipLoadError, MembershipReadFailure};
use crate::membership_load::index::{MembershipIndex, SharedEntry, SharedMembershipIndex};
use crate::membership_load::load::MembershipLoad;
use crate::membership_load::rows::{
    MembershipFilter, MembershipInclude, MembershipRowError, TeamMembershipRows,
};

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(crate) fn key(user_id: &str, team_id: &str) -> String {
    CacheKey::TeamMembershipReservation { user_id, team_id }.render()
}

pub(crate) fn membership() -> LiteLLMTeamMembership {
    LiteLLMTeamMembership {
        user_id: "user-1".to_owned(),
        team_id: "team-1".to_owned(),
        budget_id: None,
        spend: Counter(Some(Decimal::new(125, 2))),
        total_spend: Counter(Some(Decimal::new(250, 2))),
        litellm_budget_table: None,
    }
}

pub(crate) fn stored_membership() -> Json {
    payload::encode(&membership()).expect("the membership encodes")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WriteKind {
    Membership,
    Absent,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RecordedWrite {
    pub key: String,
    pub kind: WriteKind,
    pub ttl: Option<f64>,
}

// Two tiers like B733: a local read sees the memory map only, and any other read falls through to Redis.
pub(crate) struct FakeCache {
    local: Mutex<BTreeMap<String, Json>>,
    redis: Mutex<BTreeMap<String, Json>>,
    reads: Mutex<Vec<(String, bool)>>,
    writes: Mutex<Vec<RecordedWrite>>,
    failing_read: Mutex<Option<bool>>,
    failing_write: AtomicBool,
    management_ttl: f64,
}

impl FakeCache {
    pub(crate) fn new(management_ttl: f64) -> Arc<Self> {
        Arc::new(Self {
            local: Mutex::new(BTreeMap::new()),
            redis: Mutex::new(BTreeMap::new()),
            reads: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
            failing_read: Mutex::new(None),
            failing_write: AtomicBool::new(false),
            management_ttl,
        })
    }

    pub(crate) fn seed_local(&self, key: &str, stored: Json) {
        lock(&self.local).insert(key.to_owned(), stored);
    }

    pub(crate) fn seed_redis(&self, key: &str, stored: Json) {
        lock(&self.redis).insert(key.to_owned(), stored);
    }

    pub(crate) fn reads(&self) -> Vec<(String, bool)> {
        lock(&self.reads).clone()
    }

    pub(crate) fn writes(&self) -> Vec<RecordedWrite> {
        lock(&self.writes).clone()
    }

    pub(crate) fn fail_reads(&self, local_only: bool) {
        *lock(&self.failing_read) = Some(local_only);
    }

    pub(crate) fn fail_writes(&self) {
        self.failing_write.store(true, Ordering::SeqCst);
    }

    fn record_write(
        &self,
        key: &str,
        kind: WriteKind,
        ttl: Option<f64>,
    ) -> Result<(), AuthCacheError> {
        if self.failing_write.load(Ordering::SeqCst) {
            return Err(cache_failure());
        }
        lock(&self.writes).push(RecordedWrite {
            key: key.to_owned(),
            kind,
            ttl,
        });
        Ok(())
    }
}

#[async_trait]
impl MembershipCache for FakeCache {
    fn management_ttl(&self) -> f64 {
        self.management_ttl
    }

    async fn read(&self, key: &str, local_only: bool) -> Result<Option<Json>, AuthCacheError> {
        lock(&self.reads).push((key.to_owned(), local_only));
        if *lock(&self.failing_read) == Some(local_only) {
            return Err(cache_failure());
        }
        if let Some(hit) = lock(&self.local).get(key) {
            return Ok(Some(hit.clone()));
        }
        if local_only {
            return Ok(None);
        }
        Ok(lock(&self.redis).get(key).cloned())
    }

    async fn write_membership(
        &self,
        key: &str,
        _membership: &LiteLLMTeamMembership,
        ttl: Option<f64>,
    ) -> Result<(), AuthCacheError> {
        self.record_write(key, WriteKind::Membership, ttl)
    }

    async fn write_absent(&self, key: &str, ttl: Option<f64>) -> Result<(), AuthCacheError> {
        self.record_write(key, WriteKind::Absent, ttl)
    }
}

pub(crate) fn cache_failure() -> AuthCacheError {
    AuthCacheError::Payload(Box::new(PayloadError::Json(serde_json::Error::io(
        io::Error::other("the cache backend failed"),
    ))))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowsAnswer {
    Found,
    Missing,
    Database,
    Validation,
}

pub(crate) struct FakeRows {
    answer: RowsAnswer,
    blocked: AtomicBool,
    calls: AtomicUsize,
    completed: AtomicBool,
    started: Semaphore,
    released: Semaphore,
    last: Mutex<Option<(String, String)>>,
}

impl FakeRows {
    pub(crate) fn new(answer: RowsAnswer) -> Arc<Self> {
        Arc::new(Self {
            answer,
            blocked: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            completed: AtomicBool::new(false),
            started: Semaphore::new(0),
            released: Semaphore::new(0),
            last: Mutex::new(None),
        })
    }

    pub(crate) fn blocked(answer: RowsAnswer) -> Arc<Self> {
        let rows = Self::new(answer);
        rows.blocked.store(true, Ordering::SeqCst);
        rows
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub(crate) fn completed(&self) -> bool {
        self.completed.load(Ordering::SeqCst)
    }

    pub(crate) fn last_call(&self) -> Option<(String, String)> {
        lock(&self.last).clone()
    }

    pub(crate) async fn wait_started(&self) {
        self.started
            .acquire()
            .await
            .expect("the started gate stays open")
            .forget();
    }

    pub(crate) fn release(&self) {
        self.released.add_permits(1);
    }
}

#[async_trait]
impl TeamMembershipRows for FakeRows {
    async fn find_membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipReadFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *lock(&self.last) = Some((user_id.to_owned(), team_id.to_owned()));
        self.started.add_permits(1);
        if self.blocked.load(Ordering::SeqCst) {
            self.released
                .acquire()
                .await
                .expect("the released gate stays open")
                .forget();
        }
        self.completed.store(true, Ordering::SeqCst);
        match self.answer {
            RowsAnswer::Found => Ok(Some(membership())),
            RowsAnswer::Missing => Ok(None),
            RowsAnswer::Database => Err(read_failure(AuthReadError::Lookup(
                DbLookupError::Database(database_fault()),
            ))),
            RowsAnswer::Validation => Err(read_failure(AuthReadError::Repository(
                DbError::InvalidRecord {
                    source: MembershipRowError::Record(invalid_json()),
                },
            ))),
        }
    }
}

pub(crate) fn read_failure(source: AuthReadError<MembershipRowError>) -> MembershipReadFailure {
    MembershipReadFailure {
        source: Box::new(source),
    }
}

pub(crate) fn database_fault() -> FaultDbError {
    FaultDbError {
        error_type: ErrorType::Read,
        message: "the membership read failed".to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

pub(crate) fn invalid_json() -> serde_json::Error {
    serde_json::from_str::<()>("{").expect_err("the malformed record")
}

// The source index accepts any object, but only an asyncio.Task joins; this index holds the other shape.
pub(crate) struct NonTaskIndex;

impl MembershipIndex for NonTaskIndex {
    fn entry(&self, _key: &str) -> Option<SharedEntry> {
        Some(SharedEntry::Foreign)
    }

    fn install(&self, _key: &str, _load: InflightLoad<MembershipLoad>) {}

    fn remove_if(&self, _key: &str, _id: Id) {}
}

pub(crate) fn index() -> (
    Arc<MembershipInflight<MembershipLoad>>,
    Arc<dyn MembershipIndex>,
) {
    let registry = Arc::new(MembershipInflight::default());
    let shared: Arc<dyn MembershipIndex> =
        Arc::new(SharedMembershipIndex::new(Arc::clone(&registry)));
    (registry, shared)
}

// Counts joins by releasing a permit whenever the index is asked for an entry.
pub(crate) struct CountingIndex {
    inner: Arc<dyn MembershipIndex>,
    pub(crate) entries: Semaphore,
}

impl CountingIndex {
    pub(crate) fn new(inner: Arc<dyn MembershipIndex>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            entries: Semaphore::new(0),
        })
    }

    pub(crate) async fn wait_entry(&self) {
        self.entries
            .acquire()
            .await
            .expect("the entry gate stays open")
            .forget();
    }
}

impl MembershipIndex for CountingIndex {
    fn entry(&self, key: &str) -> Option<SharedEntry> {
        self.entries.add_permits(1);
        self.inner.entry(key)
    }

    fn install(&self, key: &str, load: InflightLoad<MembershipLoad>) {
        self.inner.install(key, load);
    }

    fn remove_if(&self, key: &str, id: Id) {
        self.inner.remove_if(key, id);
    }
}

pub(crate) struct SilentTransport;

#[async_trait]
impl InvalidationTransport for SilentTransport {
    async fn publish(&self, _cache_key: &str, _new_value: Option<f64>, _ttl: Option<f64>) {}

    async fn evict_and_broadcast(&self, _cache_keys: &[String], _local: &dyn LocalEviction) {}
}

pub(crate) struct ResetRig {
    pub(crate) invalidation: AuthInvalidation,
    pub(crate) spend: Arc<DualCache<LocalMemoryOps>>,
}

fn clock() -> Arc<dyn Fn() -> f64 + Send + Sync> {
    Arc::new(|| 1_000.0_f64)
}

pub(crate) fn local_memory() -> Arc<LocalMemoryOps> {
    Arc::new(LocalMemoryOps::new(1024_i64, clock()).expect("the local memory cache builds"))
}

fn dual_settings() -> DualCacheSettings {
    DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    }
}

pub(crate) fn reset_rig() -> ResetRig {
    let spend = Arc::new(DualCache::new(local_memory(), None, dual_settings()));
    let auth: Arc<dyn AuthInvalidationCache> = Arc::new(gateway_cache(
        local_memory(),
        local_memory(),
        None,
        dual_settings(),
        60.0_f64,
    ));
    let counters: Arc<dyn SpendCounterCache> = Arc::clone(&spend);
    ResetRig {
        invalidation: AuthInvalidation {
            auth,
            usage: None,
            spend: counters,
            transport: Arc::new(SilentTransport),
            clock: Arc::new(SystemClock),
            access_group_ttl: 60.0_f64,
        },
        spend,
    }
}

pub(crate) fn lookup(
    cache: Arc<FakeCache>,
    rows: Option<Arc<FakeRows>>,
    index: Arc<dyn MembershipIndex>,
    tasks: &TaskTracker,
) -> MembershipLookup {
    MembershipLookup {
        cache,
        rows: rows.map(|rows| -> Arc<dyn TeamMembershipRows> { rows }),
        index,
        tasks: tasks.clone(),
    }
}

pub(crate) fn expect_present(
    result: Result<Option<LiteLLMTeamMembership>, MembershipLoadError>,
) -> LiteLLMTeamMembership {
    result
        .expect("the membership lookup succeeds")
        .expect("the membership is present")
}

pub(crate) fn expect_absent(result: Result<Option<LiteLLMTeamMembership>, MembershipLoadError>) {
    assert_eq!(result.expect("the membership lookup succeeds"), None);
}

pub(crate) fn expect_error(
    result: Result<Option<LiteLLMTeamMembership>, MembershipLoadError>,
) -> MembershipLoadError {
    match result {
        Ok(_membership) => panic!("the membership lookup must fail"),
        Err(error) => error,
    }
}

pub(crate) fn shared(error: MembershipLoadError) -> Arc<MembershipLoadError> {
    match error {
        MembershipLoadError::Shared(shared) => shared,
        other => panic!("the waiter sees the shared failure, got {other}"),
    }
}

pub(crate) async fn drain(tasks: &TaskTracker) {
    tasks.close();
    tasks.wait().await;
}

// A reset pops the load before it waits on it, so the registry empties on the reset task's first poll.
pub(crate) async fn wait_until_popped(registry: &MembershipInflight<MembershipLoad>, key: &str) {
    for _ in 0..100_u8 {
        if registry.get(key).is_none() {
            return;
        }
        yield_now().await;
    }
}

#[derive(Clone)]
pub(crate) struct ManualClock(Arc<Mutex<Instant>>);

impl Default for ManualClock {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Instant::now())))
    }
}

impl LookupClock for ManualClock {
    fn now(&self) -> Instant {
        *lock(&self.0)
    }
}

pub(crate) struct Harness {
    pub(crate) tracker: Arc<DbLookupStallTracker<ManualClock>>,
    pub(crate) tasks: TaskTracker,
}

impl Harness {
    pub(crate) fn new() -> Self {
        let tracker = Arc::new(DbLookupStallTracker {
            clock: ManualClock::default(),
            last_hit: Mutex::new(None),
        });
        Self {
            tracker,
            tasks: TaskTracker::new(),
        }
    }

    pub(crate) fn context(&self, default_deadline_seconds: f64) -> ReadContext<ManualClock> {
        ReadContext {
            default_deadline_seconds,
            tracker: Arc::clone(&self.tracker),
            tasks: self.tasks.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TableMode {
    Row,
    Elapsed,
}

pub(crate) struct RecordingTable {
    mode: TableMode,
    request: Mutex<Option<FindOne<MembershipFilter, MembershipInclude>>>,
}

impl RecordingTable {
    pub(crate) fn new(mode: TableMode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            request: Mutex::new(None),
        })
    }

    pub(crate) fn request(&self) -> Option<FindOne<MembershipFilter, MembershipInclude>> {
        lock(&self.request).clone()
    }
}

type OneAnswer = Result<Option<LiteLLMTeamMembership>, AuthReadError<MembershipRowError>>;

#[async_trait]
impl AuthTable for RecordingTable {
    type Row = LiteLLMTeamMembership;
    type Key = MembershipFilter;
    type Filter = MembershipFilter;
    type Include = MembershipInclude;
    type Data = ();
    type Failure = MembershipRowError;

    async fn find_unique(&self, _request: FindOne<Self::Key, Self::Include>) -> OneAnswer {
        panic!("the membership lookup reads with find_first")
    }

    async fn find_first(&self, request: FindOne<Self::Filter, Self::Include>) -> OneAnswer {
        *lock(&self.request) = Some(request);
        match self.mode {
            TableMode::Row => Ok(Some(membership())),
            TableMode::Elapsed => {
                let elapsed = timeout(Duration::ZERO, pending::<()>())
                    .await
                    .expect_err("the zero timeout elapses");
                Err(AuthReadError::Lookup(DbLookupError::Elapsed(elapsed)))
            }
        }
    }

    async fn find_many(
        &self,
        _request: FindMany<Self::Filter, Self::Include>,
    ) -> Result<Vec<LiteLLMTeamMembership>, AuthReadError<MembershipRowError>> {
        panic!("the membership lookup reads one row")
    }

    async fn update(&self, _request: Update<Self::Key, Self::Data>) -> OneAnswer {
        panic!("the membership lookup writes nothing")
    }

    async fn create(
        &self,
        _request: Create<Self::Data, Self::Include>,
    ) -> Result<LiteLLMTeamMembership, AuthReadError<MembershipRowError>> {
        panic!("the membership lookup writes nothing")
    }
}

#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    pub(crate) fn capture() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .with_max_level(Level::TRACE)
            .with_writer(logs.clone())
            .finish();
        (logs, set_default(subscriber))
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8(lock(&self.0).clone()).expect("the captured logs are UTF-8")
    }
}

impl io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        lock(&self.0).extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}
