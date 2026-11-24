#![cfg(test)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use betterlite_redis_fixture::server::RedisFixture;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::gateway_request_tracking::test_support::captured_logs;
use crate::queues::pod_lock::PodLockManager;
use crate::queues::pod_lock::cache::RedisLockCache;
use crate::queues::pod_lock::owner::lease_key;
use crate::redis_spend::buffer::RedisUpdateBuffer;
use crate::redis_spend::codec::{decode_daily_batch, decode_entity_batch};
use crate::redis_spend::keys::{
    REDIS_DAILY_USER_BUFFER_KEY, REDIS_UPDATE_BUFFER_KEY, REDIS_WINDOW_BUFFER_KEY,
};
use crate::redis_spend::lists::BufferReply;
use crate::redis_spend::publication::RedisSpendPublisher;
use crate::search_tools::create::dump::safe_dumps;
use crate::spend_commit::policy::{SpendFailure, SpendRetryPolicy};
use crate::spend_commit::tables::SpendTable;
use crate::spend_flush::daily::DailyCommitParts;
use crate::spend_flush::fakes::tests::{
    ChunkStep, Events, FakeCache, FakeDailyWriter, FakeEntityWriter, FakeWindow, FixedClock,
    FixedJitter,
};
use crate::spend_flush::redis::DB_SPEND_UPDATE_JOB_NAME;
use crate::spend_flush::redis::fakes::tests::{
    CountingPause, Queues, TableFailWriter, UnreachableLists, buffer as redis_buffer, buffer_over,
    cache_for, daily_user_rows, entity_batch, length, parts, parts_with_pause, pod, publisher_over,
    push, range, raw, redis_publisher, spend_queues, window_payload,
};
use crate::spend_flush::redis::jobs::SPEND_JOB_RETRIES;
use crate::spend_flush::redis::leader::{RedisFlushError, RedisSpendFlush};
use crate::spend_log_storage::rows::writer::NoSpendLogFailureSink;
use crate::spend_queues::entity::EntityType;

// One pod's flusher over a live Redis: the queues it publishes, the buffer it drains, its own lease, and the fake ports its commit drives.
struct LeaderCase {
    queues: Queues,
    events: Arc<Events>,
    entity_writer: Arc<FakeEntityWriter>,
    daily: Arc<FakeDailyWriter>,
    window: Arc<FakeWindow>,
    cache_reset: FakeCache,
    publisher: RedisSpendPublisher,
    buffer: RedisUpdateBuffer,
    lease: PodLockManager<RedisLockCache>,
    clock: FixedClock,
    jitter: FixedJitter,
    pause: Arc<CountingPause>,
    failures: NoSpendLogFailureSink,
    parts: Arc<DailyCommitParts<FakeDailyWriter>>,
}

impl LeaderCase {
    async fn new(fixture: &RedisFixture, steps: Vec<ChunkStep>) -> Self {
        let cache = cache_for(fixture).await;
        let events = Arc::new(Events::default());
        let entity_writer = Arc::new(FakeEntityWriter::new(Arc::clone(&events)));
        let daily = Arc::new(FakeDailyWriter::new(Arc::clone(&events), steps));
        let window = Arc::new(FakeWindow::new(Arc::clone(&events)));
        let pause = Arc::new(CountingPause::default());
        let parts = parts_with_pause(Arc::clone(&daily), Arc::clone(&pause));
        Self {
            queues: spend_queues(),
            cache_reset: FakeCache::new(Arc::clone(&events)),
            publisher: redis_publisher(&cache),
            buffer: redis_buffer(&cache),
            lease: pod(&cache),
            clock: FixedClock {
                instant: Instant::now(),
            },
            jitter: FixedJitter,
            failures: NoSpendLogFailureSink,
            entity_writer,
            daily,
            window,
            parts,
            pause,
            events,
        }
    }

    fn policy(&self) -> SpendRetryPolicy<'_> {
        SpendRetryPolicy {
            retries: 0,
            clock: &self.clock,
            jitter: &self.jitter,
            pause: &*self.pause,
            failures: &self.failures,
        }
    }

    async fn flush(&self) -> Result<(), RedisFlushError> {
        let flusher = RedisSpendFlush {
            queues: self.queues.view(),
            publisher: &self.publisher,
            buffer: &self.buffer,
            lease: &self.lease,
            entity_writer: &*self.entity_writer,
            entity_cache: &self.cache_reset,
            policy: self.policy(),
            daily_parts: &self.parts,
            window: &*self.window,
        };
        flusher.commit_from_redis(SPEND_JOB_RETRIES).await
    }
}

#[tokio::test]
async fn only_the_lease_holder_drains_and_a_refused_pod_keeps_its_published_cohort() {
    let fixture = RedisFixture::start().await.unwrap();
    let holder = Arc::new(LeaderCase::new(&fixture, vec![ChunkStep::WaitThenCommit]).await);
    holder
        .queues
        .add_entity(EntityType::User, "u-holder", "1.5")
        .await;
    holder.queues.add_daily_user("2026-01-02", "1").await;
    let refused = LeaderCase::new(&fixture, vec![ChunkStep::Commit]).await;
    refused
        .queues
        .add_entity(EntityType::User, "u-refused", "1.25")
        .await;

    let mut tasks = JoinSet::new();
    let flushing = Arc::clone(&holder);
    tasks.spawn(async move { flushing.flush().await });
    timeout(Duration::from_secs(5), holder.daily.entered().notified())
        .await
        .unwrap();

    refused.flush().await.unwrap();
    holder.daily.release().notify_one();
    tasks.join_next().await.unwrap().unwrap().unwrap();

    assert_eq!(
        holder.events.entries(),
        vec![
            "entity user_list_transactions 1".to_owned(),
            "daily User 1 WaitThenCommit".to_owned(),
        ]
    );
    assert!(
        refused.events.entries().is_empty(),
        "the refused pod drained: {:?}",
        refused.events.entries()
    );
    let mut connection = raw(&fixture).await;
    assert_eq!(
        length(&mut connection, REDIS_UPDATE_BUFFER_KEY).await,
        1,
        "the refused pod's published cohort must survive for the holder's next tick"
    );
    assert_eq!(
        length(&mut connection, REDIS_DAILY_USER_BUFFER_KEY).await,
        0
    );
    assert_eq!(
        length(&mut connection, &lease_key(DB_SPEND_UPDATE_JOB_NAME)).await,
        0
    );
}

#[tokio::test]
async fn a_partial_entity_commit_restores_only_the_uncommitted_tables() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let mut connection = raw(&fixture).await;
    push(
        &mut connection,
        REDIS_UPDATE_BUFFER_KEY,
        &[safe_dumps(&entity_batch()).unwrap()],
    )
    .await;

    let events = Arc::new(Events::default());
    let writer = TableFailWriter::new(SpendTable::TeamMemberList, SpendFailure::Deadlock);
    let clock = FixedClock {
        instant: Instant::now(),
    };
    let jitter = FixedJitter;
    let pause = CountingPause::default();
    let failures = NoSpendLogFailureSink;
    let cache_reset = FakeCache::new(Arc::clone(&events));
    let daily = Arc::new(FakeDailyWriter::new(Arc::clone(&events), Vec::new()));
    let parts = parts(Arc::clone(&daily));
    let window = FakeWindow::new(Arc::clone(&events));
    let publisher = redis_publisher(&cache);
    let buffer = redis_buffer(&cache);
    let lease = pod(&cache);
    let queues = spend_queues();
    let policy = SpendRetryPolicy {
        retries: 0,
        clock: &clock,
        jitter: &jitter,
        pause: &pause,
        failures: &failures,
    };
    let flusher = RedisSpendFlush {
        queues: queues.view(),
        publisher: &publisher,
        buffer: &buffer,
        lease: &lease,
        entity_writer: &writer,
        entity_cache: &cache_reset,
        policy,
        daily_parts: &parts,
        window: &window,
    };
    let (logs, _guard) = captured_logs();
    flusher.commit_from_redis(SPEND_JOB_RETRIES).await.unwrap();

    assert!(
        logs.text()
            .contains("Spend tracking - committing spend updates from Redis to DB"),
        "the holder reports the counts of the aggregate it popped"
    );
    assert_eq!(
        writer.written(),
        vec![
            SpendTable::UserList,
            SpendTable::EndUserList,
            SpendTable::KeyList,
            SpendTable::TeamList,
        ],
        "the source order stops at the table in flight"
    );
    let restored = range(&mut connection, REDIS_UPDATE_BUFFER_KEY, 0, -1).await;
    assert_eq!(restored.len(), 1);
    let leftover = decode_entity_batch(restored[0].as_bytes()).unwrap();
    assert!(leftover.user_list_transactions.is_empty());
    assert!(leftover.end_user_list_transactions.is_empty());
    assert!(leftover.key_list_transactions.is_empty());
    assert!(leftover.team_list_transactions.is_empty());
    assert!(
        leftover
            .team_member_list_transactions
            .contains_key("t1::u1"),
        "a requeue-safe failure keeps the table in flight"
    );
    assert!(leftover.org_list_transactions.contains_key("o 1"));
    assert!(leftover.project_list_transactions.contains_key("p1"));
    assert_eq!(
        length(&mut connection, &lease_key(DB_SPEND_UPDATE_JOB_NAME)).await,
        0
    );
}

#[tokio::test]
async fn an_unsafe_entity_failure_drops_the_table_in_flight_from_the_restore() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let mut connection = raw(&fixture).await;
    push(
        &mut connection,
        REDIS_UPDATE_BUFFER_KEY,
        &[safe_dumps(&entity_batch()).unwrap()],
    )
    .await;

    let events = Arc::new(Events::default());
    let writer = TableFailWriter::new(SpendTable::TeamMemberList, SpendFailure::Rejected);
    let clock = FixedClock {
        instant: Instant::now(),
    };
    let jitter = FixedJitter;
    let pause = CountingPause::default();
    let failures = NoSpendLogFailureSink;
    let cache_reset = FakeCache::new(Arc::clone(&events));
    let daily = Arc::new(FakeDailyWriter::new(Arc::clone(&events), Vec::new()));
    let parts = parts(Arc::clone(&daily));
    let window = FakeWindow::new(Arc::clone(&events));
    let publisher = redis_publisher(&cache);
    let buffer = redis_buffer(&cache);
    let lease = pod(&cache);
    let queues = spend_queues();
    let policy = SpendRetryPolicy {
        retries: 0,
        clock: &clock,
        jitter: &jitter,
        pause: &pause,
        failures: &failures,
    };
    let flusher = RedisSpendFlush {
        queues: queues.view(),
        publisher: &publisher,
        buffer: &buffer,
        lease: &lease,
        entity_writer: &writer,
        entity_cache: &cache_reset,
        policy,
        daily_parts: &parts,
        window: &window,
    };
    flusher.commit_from_redis(SPEND_JOB_RETRIES).await.unwrap();

    let restored = range(&mut connection, REDIS_UPDATE_BUFFER_KEY, 0, -1).await;
    assert_eq!(restored.len(), 1);
    let leftover = decode_entity_batch(restored[0].as_bytes()).unwrap();
    assert!(
        leftover.team_member_list_transactions.is_empty(),
        "an unsafe failure never sends the table in flight again"
    );
    assert!(leftover.org_list_transactions.contains_key("o 1"));
    assert!(leftover.project_list_transactions.contains_key("p1"));
    assert_eq!(
        length(&mut connection, &lease_key(DB_SPEND_UPDATE_JOB_NAME)).await,
        0
    );
}

#[tokio::test]
async fn a_partial_daily_commit_restores_the_remaining_rows() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let mut connection = raw(&fixture).await;
    push(
        &mut connection,
        REDIS_DAILY_USER_BUFFER_KEY,
        &[safe_dumps(&daily_user_rows(150)).unwrap()],
    )
    .await;

    let events = Arc::new(Events::default());
    let daily = Arc::new(FakeDailyWriter::new(
        Arc::clone(&events),
        vec![
            ChunkStep::Commit,
            ChunkStep::Fail(SpendFailure::NotSent),
            ChunkStep::Fail(SpendFailure::NotSent),
            ChunkStep::Fail(SpendFailure::NotSent),
            ChunkStep::Fail(SpendFailure::NotSent),
        ],
    ));
    let pause = Arc::new(CountingPause::default());
    let parts = parts_with_pause(Arc::clone(&daily), Arc::clone(&pause));
    let entity_writer = FakeEntityWriter::new(Arc::clone(&events));
    let cache_reset = FakeCache::new(Arc::clone(&events));
    let window = FakeWindow::new(Arc::clone(&events));
    let publisher = redis_publisher(&cache);
    let buffer = redis_buffer(&cache);
    let lease = pod(&cache);
    let queues = spend_queues();
    let clock = FixedClock {
        instant: Instant::now(),
    };
    let jitter = FixedJitter;
    let failures = NoSpendLogFailureSink;
    let policy = SpendRetryPolicy {
        retries: 0,
        clock: &clock,
        jitter: &jitter,
        pause: &*pause,
        failures: &failures,
    };
    let flusher = RedisSpendFlush {
        queues: queues.view(),
        publisher: &publisher,
        buffer: &buffer,
        lease: &lease,
        entity_writer: &entity_writer,
        entity_cache: &cache_reset,
        policy,
        daily_parts: &parts,
        window: &window,
    };
    flusher.commit_from_redis(SPEND_JOB_RETRIES).await.unwrap();

    let entries = events.entries();
    assert_eq!(
        entries.first().map(String::as_str),
        Some("daily User 100 Commit"),
        "the batch size commits the first hundred rows as one chunk"
    );
    let failed = entries
        .iter()
        .filter(|event| event.as_str() == "daily User 50 Fail(NotSent)")
        .count();
    assert_eq!(
        failed, 4,
        "three retries make four attempts of the chunk the commit kept"
    );
    assert_eq!(pause.delays().len(), 3);
    let restored = range(&mut connection, REDIS_DAILY_USER_BUFFER_KEY, 0, -1).await;
    assert_eq!(restored.len(), 1);
    assert_eq!(
        decode_daily_batch(restored[0].as_bytes()).unwrap().len(),
        50,
        "the keys the failed second chunk kept are restored"
    );
    assert_eq!(
        length(&mut connection, &lease_key(DB_SPEND_UPDATE_JOB_NAME)).await,
        0
    );
}

#[tokio::test]
async fn an_unsafe_window_failure_drops_the_cohort_and_releases_the_lease() {
    let fixture = RedisFixture::start().await.unwrap();
    let case = LeaderCase::new(&fixture, Vec::new()).await;
    case.window.fail_with(SpendFailure::Ambiguous);
    let mut connection = raw(&fixture).await;
    push(
        &mut connection,
        REDIS_WINDOW_BUFFER_KEY,
        &[window_payload()],
    )
    .await;
    let (logs, _guard) = captured_logs();

    case.flush().await.unwrap();

    assert!(logs.text().contains("dropped 1 budget window increments"));
    assert_eq!(
        length(&mut connection, REDIS_WINDOW_BUFFER_KEY).await,
        0,
        "an ambiguous window failure must not be sent again"
    );
    assert_eq!(
        length(&mut connection, &lease_key(DB_SPEND_UPDATE_JOB_NAME)).await,
        0
    );
}

#[tokio::test]
async fn a_restore_that_cannot_reach_redis_logs_the_loss_and_releases_the_lease() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let lists = Arc::new(UnreachableLists::default());
    let encoded = safe_dumps(&entity_batch()).unwrap();
    lists.script_pipeline(vec![BufferReply::List(vec![encoded.into_bytes()])]);

    let events = Arc::new(Events::default());
    let entity_writer = FakeEntityWriter::new(Arc::clone(&events));
    entity_writer.fail_with(SpendFailure::Deadlock);
    let daily = Arc::new(FakeDailyWriter::new(Arc::clone(&events), Vec::new()));
    let window = FakeWindow::new(Arc::clone(&events));
    let cache_reset = FakeCache::new(Arc::clone(&events));
    let publisher = publisher_over(&lists);
    let buffer = buffer_over(&lists);
    let lease = pod(&cache);
    let queues = spend_queues();
    let parts = parts(Arc::clone(&daily));
    let clock = FixedClock {
        instant: Instant::now(),
    };
    let jitter = FixedJitter;
    let pause = CountingPause::default();
    let failures = NoSpendLogFailureSink;
    let policy = SpendRetryPolicy {
        retries: 0,
        clock: &clock,
        jitter: &jitter,
        pause: &pause,
        failures: &failures,
    };
    let (logs, _guard) = captured_logs();

    let flusher = RedisSpendFlush {
        queues: queues.view(),
        publisher: &publisher,
        buffer: &buffer,
        lease: &lease,
        entity_writer: &entity_writer,
        entity_cache: &cache_reset,
        policy,
        daily_parts: &parts,
        window: &window,
    };
    flusher.commit_from_redis(SPEND_JOB_RETRIES).await.unwrap();

    assert!(logs.text().contains("These spend updates are lost"));
    assert_eq!(
        lists.push_attempts(),
        2,
        "the publish attempt and the failed restore are the only pushes"
    );
    let mut connection = raw(&fixture).await;
    assert_eq!(
        length(&mut connection, &lease_key(DB_SPEND_UPDATE_JOB_NAME)).await,
        0,
        "a restore that cannot reach redis must not keep the lease"
    );
}

#[tokio::test]
async fn a_drain_error_releases_the_lease_without_restoring_anything() {
    let fixture = RedisFixture::start().await.unwrap();
    let case = LeaderCase::new(&fixture, Vec::new()).await;
    let mut connection = raw(&fixture).await;
    push(
        &mut connection,
        REDIS_UPDATE_BUFFER_KEY,
        &["{not json".to_owned()],
    )
    .await;
    let (logs, _guard) = captured_logs();

    case.flush().await.unwrap();

    assert!(
        logs.text()
            .contains("failed to commit spend updates from Redis to DB")
    );
    assert_eq!(length(&mut connection, REDIS_UPDATE_BUFFER_KEY).await, 0);
    assert_eq!(
        length(&mut connection, &lease_key(DB_SPEND_UPDATE_JOB_NAME)).await,
        0
    );
}

#[tokio::test]
async fn the_handlers_retry_count_reaches_the_redis_daily_commit() {
    let fixture = RedisFixture::start().await.unwrap();
    let case = LeaderCase::new(&fixture, vec![ChunkStep::Fail(SpendFailure::NotSent); 4]).await;
    case.queues.add_daily_user("2026-01-02", "1").await;

    case.flush().await.unwrap();

    let events = case.events.entries();
    let attempts = events
        .iter()
        .filter(|event| event.starts_with("daily User"))
        .count();
    assert_eq!(attempts, 4, "three retries make four attempts");
    assert_eq!(case.pause.delays().len(), 3);
    let mut connection = raw(&fixture).await;
    assert_eq!(
        length(&mut connection, REDIS_DAILY_USER_BUFFER_KEY).await,
        1,
        "the whole map goes back after an exhausted retryable failure"
    );
}

#[tokio::test]
async fn the_handlers_retry_count_reaches_the_redis_entity_commit() {
    let fixture = RedisFixture::start().await.unwrap();
    let case = LeaderCase::new(&fixture, Vec::new()).await;
    case.entity_writer.fail_with(SpendFailure::Deadlock);
    case.queues
        .add_entity(EntityType::User, "u-retry", "2")
        .await;

    case.flush().await.unwrap();

    let attempts = case
        .events
        .entries()
        .iter()
        .filter(|event| event.starts_with("entity user_list_transactions"))
        .count();
    assert_eq!(attempts, 4, "three retries make four entity attempts");
    assert_eq!(case.pause.delays().len(), 3);
}
