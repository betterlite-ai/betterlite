#![cfg(test)]

// The fixtures the three Redis flusher test modules share: the queues a tick publishes from, the real-Redis handles, the pod leases two pods contend on, and the wire records a case seeds.
use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use betterlite_redis::list_operations::error::RedisListOperationsError;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use betterlite_redis_fixture::server::RedisFixture;
use redis::aio::MultiplexedConnection;
use redis::{Client, cmd};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::daily_spend_sql::entities::SpendRow;
use crate::daily_spend_sql::values::SpendColumnValue;
use crate::entity_spend_commit::{EntityTableWriter, TableFuture};
use crate::queues::pod_lock::cache::RedisLockCache;
use crate::queues::pod_lock::owner::PodId;
use crate::queues::pod_lock::{NoPodLockEventSink, PodLockManager};
use crate::redis_spend::buffer::RedisUpdateBuffer;
use crate::redis_spend::events::NoRedisBufferEventSink;
use crate::redis_spend::lists::{
    BufferPop, BufferPush, BufferReply, RedisSpendLists, SpendBufferListError, SpendBufferLists,
    SpendBufferPush, SpendListFuture,
};
use crate::redis_spend::publication::{RedisSpendPublisher, SpendStaging, SpendUpdateQueues};
use crate::spend_commit::policy::{SpendFailure, SpendRetryPolicy};
use crate::spend_commit::tables::SpendTable;
use crate::spend_flush::daily::DailyCommitParts;
use crate::spend_flush::fakes::tests::{
    ChunkStep, Events, FakeCache, FakeDailyWriter, FakeEntityError, FakeEntityWriter, FakeWindow,
    FixedClock, FixedJitter,
};
use crate::spend_flush::redis::jobs::SPEND_JOB_RETRIES;
use crate::spend_flush::redis::leader::dispatch::SpendFlushPass;
use crate::spend_flush::redis::leader::{RedisFlushError, RedisSpendFlush};
use crate::spend_flush::service::ServiceFuture;
use crate::spend_log_storage::rows::writer::NoSpendLogFailureSink;
use crate::spend_queues::daily::{DailySpendUpdate, DailySpendUpdateQueue};
use crate::spend_queues::entity::{
    EntityType, SpendAmount, SpendUpdateQueue, SpendUpdateQueueItem, SpendUpdateTransactions,
};
use crate::spend_queues::events::NoSpendQueueEventSink;
use crate::tool_discovery::ToolDiscoveryQueue;
use crate::tool_spend::RetryPause;
use crate::window_queue::queue::WindowSpendUpdateQueue;

// The spend queues one pod publishes from, each unbounded enough that only a test asking for a full one meets a closed queue.
pub(crate) struct Queues {
    pub(crate) entity: Arc<SpendUpdateQueue>,
    pub(crate) daily_user: Arc<DailySpendUpdateQueue>,
    pub(crate) daily_team: Arc<DailySpendUpdateQueue>,
    pub(crate) daily_org: Arc<DailySpendUpdateQueue>,
    pub(crate) daily_end_user: Arc<DailySpendUpdateQueue>,
    pub(crate) daily_agent: Arc<DailySpendUpdateQueue>,
    pub(crate) window: Arc<WindowSpendUpdateQueue>,
    pub(crate) discovery: Arc<ToolDiscoveryQueue>,
}

pub(crate) fn spend_queues() -> Queues {
    Queues {
        entity: Arc::new(
            SpendUpdateQueue::new(100, 1000, 100, Arc::new(NoSpendQueueEventSink)).unwrap(),
        ),
        daily_user: Arc::new(daily_queue(100, 1000, 100)),
        daily_team: Arc::new(daily_queue(100, 1000, 100)),
        daily_org: Arc::new(daily_queue(100, 1000, 100)),
        daily_end_user: Arc::new(daily_queue(100, 1000, 100)),
        daily_agent: Arc::new(daily_queue(100, 1000, 100)),
        window: Arc::new(window_queue(100, 1000, 100)),
        discovery: Arc::new(ToolDiscoveryQueue::new(100).unwrap()),
    }
}

impl Queues {
    // The six memory queues B1738 publishes, in the source's order, with a window queue because the test deployment runs one.
    pub(crate) fn view(&self) -> SpendUpdateQueues<'_> {
        SpendUpdateQueues {
            entity: &self.entity,
            daily_user: &self.daily_user,
            daily_team: &self.daily_team,
            daily_org: &self.daily_org,
            daily_end_user: &self.daily_end_user,
            daily_agent: &self.daily_agent,
            window: Some(&self.window),
        }
    }

    pub(crate) async fn add_entity(&self, kind: EntityType, id: &str, cost: &str) {
        self.entity
            .add_update(queue_item(kind, id, cost))
            .await
            .unwrap();
    }

    pub(crate) async fn add_daily_user(&self, day: &str, spend: &str) {
        self.daily_user
            .add_update(daily_row(day, spend, 1))
            .await
            .unwrap();
    }
}

// A retry pause that records every wait and returns at once, so a case can count the retries a commit policy actually spent.
#[derive(Default)]
pub(crate) struct CountingPause {
    delays: Mutex<Vec<Duration>>,
}

impl CountingPause {
    pub(crate) fn delays(&self) -> Vec<Duration> {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RetryPause for CountingPause {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(delay);
        Box::pin(ready(()))
    }
}

// The daily commit parts one case reads: the writer under test, a fixed clock and jitter, and the retry pause the test inspects.
pub(crate) fn parts_with_pause<Writer>(
    writer: Arc<Writer>,
    pause: Arc<CountingPause>,
) -> Arc<DailyCommitParts<Writer>> {
    Arc::new(DailyCommitParts {
        writer,
        retries: 0,
        clock: Arc::new(FixedClock {
            instant: Instant::now(),
        }),
        jitter: Arc::new(FixedJitter),
        pause,
        failures: Arc::new(NoSpendLogFailureSink),
    })
}

fn publisher(lists: Option<Arc<dyn SpendBufferPush>>) -> RedisSpendPublisher {
    RedisSpendPublisher {
        lists,
        events: Arc::new(NoRedisBufferEventSink),
        staging: SpendStaging::default(),
    }
}

// The append port over a live Redis: the publisher pushes into the same buffer the drain pops from.
pub(crate) fn redis_publisher(cache: &Arc<RedisCache>) -> RedisSpendPublisher {
    publisher(Some(Arc::new(RedisSpendLists {
        cache: Arc::clone(cache),
    })))
}

// One pod's lease manager over a live Redis; two calls mint two owners that contend on the same lock keys.
pub(crate) fn pod(cache: &Arc<RedisCache>) -> PodLockManager<RedisLockCache> {
    PodLockManager::new(
        Some(Arc::new(RedisLockCache {
            cache: Arc::clone(cache),
        })),
        60,
        Arc::new(NoPodLockEventSink),
        PodId::try_from(Uuid::new_v4()).unwrap(),
    )
}

// A cache connected to the case's own Redis: the production settings path, with hooks and a breaker that stay out of the way.
struct NoHooks;

impl ServiceHooks for NoHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

struct OpenGate;

impl BreakerGate for OpenGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

const CACHE_TTL_SECONDS: i64 = 60;

pub(crate) async fn cache_for(fixture: &RedisFixture) -> Arc<RedisCache> {
    let url = serde_json::to_string(fixture.as_ref()).unwrap();
    let overrides: RedisOverrides = serde_json::from_str(&format!(r#"{{"url":{url}}}"#)).unwrap();
    let options = RedisCacheOptions {
        family: RedisCacheFamily::Standalone,
        overrides,
        namespace: None,
        redis_flush_size: None,
        default_ttl: CACHE_TTL_SECONDS,
        default_major_version: 7,
        store: Arc::new(RedisClientStore::new()),
        factory: Arc::new(LiveClientFactory::new(None)),
        gate: Arc::new(OpenGate),
        hooks: Arc::new(NoHooks),
    };
    let env: BTreeMap<String, String> = BTreeMap::new();
    Arc::new(connect(options, &env).await.unwrap())
}

fn buffer_lists(lists: Option<Arc<dyn SpendBufferLists>>) -> RedisUpdateBuffer {
    RedisUpdateBuffer {
        lists,
        enabled: true,
        dequeue_count: 5,
        events: Arc::new(NoRedisBufferEventSink),
    }
}

// The buffer over the same live Redis the publisher appends to.
pub(crate) fn buffer(cache: &Arc<RedisCache>) -> RedisUpdateBuffer {
    buffer_lists(Some(Arc::new(RedisSpendLists {
        cache: Arc::clone(cache),
    })))
}

pub(crate) async fn raw(fixture: &RedisFixture) -> MultiplexedConnection {
    Client::open(fixture.as_ref())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap()
}

pub(crate) async fn push(connection: &mut MultiplexedConnection, key: &str, values: &[String]) {
    let mut command = cmd("RPUSH");
    command.arg(key);
    for value in values {
        command.arg(value.as_str());
    }
    let _length: i64 = command.query_async(connection).await.unwrap();
}

pub(crate) async fn length(connection: &mut MultiplexedConnection, key: &str) -> i64 {
    cmd("LLEN").arg(key).query_async(connection).await.unwrap()
}

// Whether a key exists at all, for the keys the pod locks hold as strings rather than lists.
pub(crate) async fn exists(connection: &mut MultiplexedConnection, key: &str) -> i64 {
    cmd("EXISTS")
        .arg(key)
        .query_async(connection)
        .await
        .unwrap()
}

pub(crate) async fn range(
    connection: &mut MultiplexedConnection,
    key: &str,
    start: i64,
    stop: i64,
) -> Vec<String> {
    cmd("LRANGE")
        .arg(key)
        .arg(start)
        .arg(stop)
        .query_async(connection)
        .await
        .unwrap()
}

// One buffer's length on the case's own Redis, read over a fresh connection.
pub(crate) async fn buffer_length(fixture: &RedisFixture, key: &str) -> i64 {
    let mut connection = raw(fixture).await;
    length(&mut connection, key).await
}

// The ports one leader flush drives: the queues it publishes from, the live-Redis publisher, buffer and lease, and the fake writers its commit reads.
pub(crate) struct FlushCase<Writer> {
    pub(crate) queues: Queues,
    pub(crate) events: Arc<Events>,
    pub(crate) entity_writer: Arc<Writer>,
    pub(crate) daily: Arc<FakeDailyWriter>,
    pub(crate) window: Arc<FakeWindow>,
    pub(crate) cache_reset: FakeCache,
    pub(crate) publisher: RedisSpendPublisher,
    pub(crate) buffer: RedisUpdateBuffer,
    pub(crate) lease: PodLockManager<RedisLockCache>,
    pub(crate) clock: FixedClock,
    pub(crate) jitter: FixedJitter,
    pub(crate) pause: Arc<CountingPause>,
    pub(crate) failures: NoSpendLogFailureSink,
    pub(crate) parts: Arc<DailyCommitParts<FakeDailyWriter>>,
}

impl<Writer> FlushCase<Writer>
where
    Writer: EntityTableWriter,
{
    pub(crate) async fn new(
        fixture: &RedisFixture,
        events: Arc<Events>,
        entity_writer: Arc<Writer>,
        steps: Vec<ChunkStep>,
    ) -> Self {
        let cache = cache_for(fixture).await;
        let daily = Arc::new(FakeDailyWriter::new(Arc::clone(&events), steps));
        let pause = Arc::new(CountingPause::default());
        let parts = parts_with_pause(Arc::clone(&daily), Arc::clone(&pause));
        Self {
            queues: spend_queues(),
            window: Arc::new(FakeWindow::new(Arc::clone(&events))),
            cache_reset: FakeCache::new(Arc::clone(&events)),
            publisher: redis_publisher(&cache),
            buffer: buffer(&cache),
            lease: pod(&cache),
            clock: FixedClock {
                instant: Instant::now(),
            },
            jitter: FixedJitter,
            failures: NoSpendLogFailureSink,
            entity_writer,
            daily,
            parts,
            pause,
            events,
        }
    }

    // A publisher and buffer no tick can reach, while the lease stays on the case's real Redis so a case can read the key it releases.
    pub(crate) fn use_lists(&mut self, lists: &Arc<UnreachableLists>) {
        self.publisher = publisher_over(lists);
        self.buffer = buffer_over(lists);
    }

    // One tick of the source's Redis branch, with the retry count the job adapter pins.
    pub(crate) async fn flush(&self) -> Result<(), RedisFlushError> {
        let flusher = RedisSpendFlush {
            queues: self.queues.view(),
            publisher: &self.publisher,
            buffer: &self.buffer,
            lease: &self.lease,
            entity_writer: &*self.entity_writer,
            entity_cache: &self.cache_reset,
            policy: SpendRetryPolicy {
                retries: 0,
                clock: &self.clock,
                jitter: &self.jitter,
                pause: &*self.pause,
                failures: &self.failures,
            },
            daily_parts: &self.parts,
            window: &*self.window,
        };
        flusher.commit_from_redis(SPEND_JOB_RETRIES).await
    }
}

// One pod over the live Redis a case started, with the fake entity writer every ordinary leader case drives.
pub(crate) async fn leader_case(
    fixture: &RedisFixture,
    steps: Vec<ChunkStep>,
) -> FlushCase<FakeEntityWriter> {
    let events = Arc::new(Events::default());
    let writer = Arc::new(FakeEntityWriter::new(Arc::clone(&events)));
    FlushCase::new(fixture, events, writer, steps).await
}

// A buffer port no append can reach: the scripted pops hand back what the holder drained, and every push fails the way an unreachable Redis does.
#[derive(Default)]
pub(crate) struct UnreachableLists {
    pipelines: Mutex<VecDeque<Vec<BufferReply>>>,
    pushes: Mutex<usize>,
}

impl UnreachableLists {
    pub(crate) fn script_pipeline(&self, replies: Vec<BufferReply>) {
        self.pipelines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(replies);
    }

    pub(crate) fn push_attempts(&self) -> usize {
        *self.pushes.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn count_push(&self) {
        let mut pushes = self.pushes.lock().unwrap_or_else(PoisonError::into_inner);
        *pushes = pushes.saturating_add(1);
    }

    fn failed_push(&self, operation: &'static str) -> SpendListFuture<'_, i64> {
        self.count_push();
        Box::pin(ready(Err(list_failure(operation))))
    }
}

fn list_failure(operation: &'static str) -> SpendBufferListError {
    SpendBufferListError::Redis {
        operation,
        source: RedisListOperationsError::UnexpectedReply { operation },
    }
}

impl SpendBufferLists for UnreachableLists {
    fn pop<'call>(
        &'call self,
        _key: &'call str,
        _count: Option<i64>,
    ) -> SpendListFuture<'call, BufferReply> {
        Box::pin(ready(Ok(BufferReply::Missing)))
    }

    fn pop_pipeline<'call>(
        &'call self,
        _operations: &'call [BufferPop],
    ) -> SpendListFuture<'call, Vec<BufferReply>> {
        let replies = self
            .pipelines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_default();
        Box::pin(ready(Ok(replies)))
    }

    fn push_and_trim<'call>(
        &'call self,
        _key: &'call str,
        _values: &'call [String],
        _max_len: i64,
    ) -> SpendListFuture<'call, i64> {
        self.failed_push("async_rpush_trim")
    }
}

impl SpendBufferPush for UnreachableLists {
    fn rpush<'call>(
        &'call self,
        _key: &'call str,
        _values: &'call [String],
    ) -> SpendListFuture<'call, i64> {
        self.failed_push("async_rpush")
    }

    fn rpush_pipeline<'call>(
        &'call self,
        _operations: &'call [BufferPush],
    ) -> SpendListFuture<'call, Vec<i64>> {
        self.count_push();
        Box::pin(ready(Err(list_failure("async_rpush_pipeline"))))
    }
}

pub(crate) fn publisher_over(lists: &Arc<UnreachableLists>) -> RedisSpendPublisher {
    let concrete: Arc<UnreachableLists> = Arc::clone(lists);
    let push_port: Arc<dyn SpendBufferPush> = concrete;
    publisher(Some(push_port))
}

pub(crate) fn buffer_over(lists: &Arc<UnreachableLists>) -> RedisUpdateBuffer {
    let concrete: Arc<UnreachableLists> = Arc::clone(lists);
    let pop_port: Arc<dyn SpendBufferLists> = concrete;
    buffer_lists(Some(pop_port))
}

// An entity writer that commits the source's table order up to one chosen table and then fails it, so a case can see exactly which tables the restore keeps.
pub(crate) struct TableFailWriter {
    failing: SpendTable,
    failure: SpendFailure,
    written: Mutex<Vec<SpendTable>>,
}

impl TableFailWriter {
    pub(crate) fn new(failing: SpendTable, failure: SpendFailure) -> Self {
        Self {
            failing,
            failure,
            written: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn written(&self) -> Vec<SpendTable> {
        self.written
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl EntityTableWriter for TableFailWriter {
    type Error = FakeEntityError;

    fn write_table<'writer>(
        &'writer self,
        table: SpendTable,
        _increments: &'writer BTreeMap<String, Decimal>,
    ) -> TableFuture<'writer, FakeEntityError> {
        if table == self.failing {
            return Box::pin(ready(Err(FakeEntityError {
                failure: self.failure,
            })));
        }
        self.written
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(table);
        Box::pin(ready(Ok(())))
    }
}

// The queues a tick publishes from and the wire rows a case seeds, built here because the redis_spend test fixtures are private to that module.
pub(crate) fn daily_queue(
    capacity: i64,
    threshold: i64,
    flush_count: i64,
) -> DailySpendUpdateQueue {
    DailySpendUpdateQueue::new(
        capacity,
        threshold,
        flush_count,
        Arc::new(NoSpendQueueEventSink),
    )
    .unwrap()
}

pub(crate) fn window_queue(
    capacity: i64,
    threshold: i64,
    flush_count: i64,
) -> WindowSpendUpdateQueue {
    WindowSpendUpdateQueue::new(capacity, threshold, flush_count).unwrap()
}

pub(crate) fn amount(value: &str) -> SpendAmount {
    SpendAmount(value.parse::<Decimal>().unwrap())
}

pub(crate) fn queue_item(kind: EntityType, id: &str, value: &str) -> SpendUpdateQueueItem {
    SpendUpdateQueueItem {
        entity_type: Some(kind),
        entity_id: Some(id.to_owned()),
        response_cost: Some(amount(value)),
    }
}

pub(crate) fn transactions(user: &[(&str, &str)]) -> SpendUpdateTransactions {
    let mut transactions = SpendUpdateTransactions::default();
    for (id, value) in user {
        transactions
            .user_list_transactions
            .insert((*id).to_owned(), amount(value));
    }
    transactions
}

pub(crate) fn daily_row(day: &str, spend: &str, prompt_tokens: i64) -> DailySpendUpdate {
    DailySpendUpdate::from([(
        day.to_owned(),
        SpendRow::from([
            (
                "spend".to_owned(),
                SpendColumnValue::Decimal(spend.parse::<Decimal>().unwrap()),
            ),
            (
                "prompt_tokens".to_owned(),
                SpendColumnValue::Int(prompt_tokens),
            ),
            ("completion_tokens".to_owned(), SpendColumnValue::Int(0)),
            ("api_requests".to_owned(), SpendColumnValue::Int(0)),
            ("successful_requests".to_owned(), SpendColumnValue::Int(0)),
            ("failed_requests".to_owned(), SpendColumnValue::Int(0)),
        ]),
    )])
}

// The one entity record the partial-commit cases seed: a row in each of the four committed tables, the table that fails, and the two after it.
pub(crate) fn entity_batch() -> SpendUpdateTransactions {
    let mut batch = transactions(&[("u1", "1.5")]);
    batch
        .end_user_list_transactions
        .insert("e1".to_owned(), amount("1.5"));
    batch
        .key_list_transactions
        .insert("k1".to_owned(), amount("1.5"));
    batch
        .team_list_transactions
        .insert("t1".to_owned(), amount("1.5"));
    batch
        .team_member_list_transactions
        .insert("team_id::t1::user_id::u1".to_owned(), amount("1.5"));
    batch
        .org_list_transactions
        .insert("o 1".to_owned(), amount("1.5"));
    batch
        .project_list_transactions
        .insert("p1".to_owned(), amount("1.5"));
    batch
}

// One daily user row per id, every column the real upsert requires, distinguishable in the table by its user_id.
pub(crate) fn daily_user_rows(count: usize) -> DailySpendUpdate {
    (0..count)
        .map(|index| {
            let user = format!("user-{index:03}");
            (user.clone(), user_spend_row(&user))
        })
        .collect()
}

pub(crate) fn user_spend_row(user: &str) -> SpendRow {
    let mut row = SpendRow::new();
    for (column, value) in [
        ("date", "2026-01-02"),
        ("user_id", user),
        ("api_key", "key-1"),
        ("model", "model-1"),
        ("custom_llm_provider", "openai"),
    ] {
        row.insert(column.to_owned(), SpendColumnValue::Text(value.to_owned()));
    }
    row.insert("spend".to_owned(), SpendColumnValue::Decimal(Decimal::ONE));
    row
}

// One window buffer record: the wire payload B1734 writes, with the spend the fake window commit reads.
pub(crate) fn window_payload() -> String {
    r#"[{"entity_type":"user","entity_id":"user-1","window_duration":"1h","window_start":"2024-01-01T00:00:00+00:00","spend":2.5}]"#.to_owned()
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted spend pass failed")]
pub(crate) struct PassError;

// A buffer-mode branch that records the retry count it was handed and succeeds or fails on script.
pub(crate) struct RecordingPass {
    name: &'static str,
    events: Arc<Events>,
    failing: bool,
}

pub(crate) fn recording_pass(
    name: &'static str,
    events: Arc<Events>,
    failing: bool,
) -> RecordingPass {
    RecordingPass {
        name,
        events,
        failing,
    }
}

impl SpendFlushPass for RecordingPass {
    type Error = PassError;

    fn commit_spend_updates(&self, retries: u32) -> ServiceFuture<'_, Result<(), PassError>> {
        self.events.push(format!("balance {} {retries}", self.name));
        let outcome: Result<(), PassError> = if self.failing { Err(PassError) } else { Ok(()) };
        Box::pin(ready(outcome))
    }
}
