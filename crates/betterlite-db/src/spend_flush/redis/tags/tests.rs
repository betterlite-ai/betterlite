#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_redis_fixture::server::RedisFixture;
use tokio::task::{JoinSet, yield_now};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::daily_spend_sql::entities::DailySpendEntity;
use crate::gateway_request_tracking::test_support::captured_logs;
use crate::queues::pod_lock::PodLockManager;
use crate::queues::pod_lock::cache::RedisLockCache;
use crate::queues::pod_lock::owner::lease_key;
use crate::redis_spend::buffer::RedisUpdateBuffer;
use crate::redis_spend::codec::decode_daily_batch;
use crate::redis_spend::keys::REDIS_DAILY_TAG_BUFFER_KEY;
use crate::redis_spend::publication::RedisSpendPublisher;
use crate::spend_commit::policy::SpendFailure;
use crate::spend_flush::daily::{DailyCommitParts, DailyFlushError};
use crate::spend_flush::fakes::tests::{ChunkStep, Events, FakeDailyError, FakeDailyWriter};
use crate::spend_flush::redis::fakes::tests::{
    CountingPause, UnreachableLists, buffer as redis_buffer, cache_for, daily_queue, daily_row,
    exists, length, parts_with_pause, pod, publisher_over, push, range, raw, redis_publisher,
};
use crate::spend_flush::redis::jobs::SPEND_JOB_RETRIES;
use crate::spend_flush::redis::tags::{
    DailyTagCommit as _, MemoryTagFlush, RedisTagFlush, TagCommitFailure as _, TagFlushError,
};
use crate::spend_flush::redis::{DAILY_TAG_SPEND_JOB_NAME, DB_SPEND_UPDATE_JOB_NAME};
use crate::spend_queues::SpendQueueError;
use crate::spend_queues::daily::DailySpendUpdateQueue;

// One pod's tag flusher over a live Redis: its own tag queue, publisher, buffer and lease, and the fake writer its commit drives.
struct TagCase {
    publisher: Arc<RedisSpendPublisher>,
    buffer: RedisUpdateBuffer,
    lease: PodLockManager<RedisLockCache>,
    queue: Arc<DailySpendUpdateQueue>,
    parts: Arc<DailyCommitParts<FakeDailyWriter>>,
    commits: TaskTracker,
    shutdown: CancellationToken,
    events: Arc<Events>,
    writer: Arc<FakeDailyWriter>,
    pause: Arc<CountingPause>,
}

impl TagCase {
    async fn new(fixture: &RedisFixture, steps: Vec<ChunkStep>) -> Self {
        let cache = cache_for(fixture).await;
        let events = Arc::new(Events::default());
        let writer = Arc::new(FakeDailyWriter::new(Arc::clone(&events), steps));
        let pause = Arc::new(CountingPause::default());
        Self {
            publisher: Arc::new(redis_publisher(&cache)),
            buffer: redis_buffer(&cache),
            lease: pod(&cache),
            queue: Arc::new(daily_queue(100, 1000, 100)),
            parts: parts_with_pause(Arc::clone(&writer), Arc::clone(&pause)),
            commits: TaskTracker::new(),
            shutdown: CancellationToken::new(),
            events,
            writer,
            pause,
        }
    }

    async fn enqueue(&self) {
        self.queue
            .add_update(daily_row("day-1", "1.5", 1))
            .await
            .unwrap();
    }

    async fn flush(&self) -> Result<(), TagFlushError<FakeDailyError>> {
        let flusher = RedisTagFlush {
            publisher: Arc::clone(&self.publisher),
            buffer: &self.buffer,
            lease: &self.lease,
            queue: &self.queue,
            parts: &self.parts,
            commits: &self.commits,
            shutdown: &self.shutdown,
        };
        flusher.commit_tag_from_redis(SPEND_JOB_RETRIES).await
    }
}

#[tokio::test]
async fn the_tag_flusher_commits_under_its_own_lock_while_the_main_spend_lock_is_held() {
    let fixture = RedisFixture::start().await.unwrap();
    let cache = cache_for(&fixture).await;
    let main_owner = pod(&cache);
    assert_eq!(
        main_owner
            .acquire_lock(DB_SPEND_UPDATE_JOB_NAME, None, true)
            .await,
        Some(true)
    );
    let case = TagCase::new(&fixture, vec![ChunkStep::Commit]).await;
    case.enqueue().await;

    case.flush().await.unwrap();

    assert_eq!(case.events.entries(), vec!["daily Tag 1 Commit".to_owned()]);
    let mut connection = raw(&fixture).await;
    assert_eq!(
        length(&mut connection, REDIS_DAILY_TAG_BUFFER_KEY).await,
        0,
        "a committed tag cohort is popped for good"
    );
    assert_eq!(
        length(&mut connection, &lease_key(DAILY_TAG_SPEND_JOB_NAME)).await,
        0
    );
    assert_eq!(
        exists(&mut connection, &lease_key(DB_SPEND_UPDATE_JOB_NAME)).await,
        1,
        "the main spend job lease belongs to the other holder and stays held"
    );
    main_owner.release_lock(DB_SPEND_UPDATE_JOB_NAME).await;
}

#[tokio::test]
async fn an_ordinary_tag_commit_failure_restores_the_remaining_rows_and_releases_the_lock() {
    let fixture = RedisFixture::start().await.unwrap();
    let case = TagCase::new(&fixture, vec![ChunkStep::Fail(SpendFailure::Other)]).await;
    case.enqueue().await;
    let (logs, _guard) = captured_logs();

    case.flush().await.unwrap();

    assert_eq!(
        case.events.entries(),
        vec!["daily Tag 1 Fail(Other)".to_owned()]
    );
    assert!(
        logs.text()
            .contains("failed to commit daily tag spend updates from Redis to DB"),
        "the outer catch logs the ordinary failure"
    );
    let mut connection = raw(&fixture).await;
    let restored = range(&mut connection, REDIS_DAILY_TAG_BUFFER_KEY, 0, -1).await;
    assert_eq!(restored.len(), 1);
    assert!(
        decode_daily_batch(restored[0].as_bytes())
            .unwrap()
            .contains_key("day-1")
    );
    assert_eq!(
        length(&mut connection, &lease_key(DAILY_TAG_SPEND_JOB_NAME)).await,
        0
    );
}

#[tokio::test]
async fn a_cancel_before_the_commit_restores_the_whole_cohort_and_propagates() {
    let fixture = RedisFixture::start().await.unwrap();
    let case = Arc::new(TagCase::new(&fixture, vec![ChunkStep::WaitThenCommit]).await);
    case.enqueue().await;
    let mut tasks = JoinSet::new();
    let flushing = Arc::clone(&case);
    tasks.spawn(async move { flushing.flush().await });
    timeout(Duration::from_secs(5), case.writer.entered().notified())
        .await
        .unwrap();

    case.shutdown.cancel();
    yield_now().await;
    case.writer.release().notify_one();
    let outcome = tasks.join_next().await.unwrap().unwrap();

    assert!(matches!(outcome, Err(TagFlushError::Cancelled)));
    assert_eq!(
        case.events.entries(),
        vec!["daily Tag 1 WaitThenCommit".to_owned()]
    );
    let mut connection = raw(&fixture).await;
    let restored = range(&mut connection, REDIS_DAILY_TAG_BUFFER_KEY, 0, -1).await;
    assert_eq!(restored.len(), 1, "the cancelled cohort goes back whole");
    assert!(
        decode_daily_batch(restored[0].as_bytes())
            .unwrap()
            .contains_key("day-1")
    );
    assert_eq!(
        length(&mut connection, &lease_key(DAILY_TAG_SPEND_JOB_NAME)).await,
        0
    );
}

#[tokio::test]
async fn a_commit_already_running_when_the_shutdown_arrives_restores_its_failed_rows() {
    let fixture = RedisFixture::start().await.unwrap();
    let case = Arc::new(
        TagCase::new(
            &fixture,
            vec![ChunkStep::BeginThenWaitThenFail(SpendFailure::Other)],
        )
        .await,
    );
    case.enqueue().await;
    let mut tasks = JoinSet::new();
    let flushing = Arc::clone(&case);
    tasks.spawn(async move { flushing.flush().await });
    timeout(Duration::from_secs(5), case.writer.entered().notified())
        .await
        .unwrap();

    case.shutdown.cancel();
    let early = tasks.join_next().await.unwrap().unwrap();
    assert!(matches!(early, Err(TagFlushError::Cancelled)));

    case.writer.release().notify_one();
    case.queue.settle_interrupted_commits().await;

    let mut connection = raw(&fixture).await;
    let restored = range(&mut connection, REDIS_DAILY_TAG_BUFFER_KEY, 0, -1).await;
    assert_eq!(
        restored.len(),
        1,
        "the tracked commit restores the rows it failed to write"
    );
    assert!(
        decode_daily_batch(restored[0].as_bytes())
            .unwrap()
            .contains_key("day-1")
    );
    assert_eq!(
        length(&mut connection, &lease_key(DAILY_TAG_SPEND_JOB_NAME)).await,
        0
    );
}

#[tokio::test]
async fn a_commit_that_finishes_while_the_shutdown_arrives_restores_nothing() {
    let fixture = RedisFixture::start().await.unwrap();
    let case = Arc::new(TagCase::new(&fixture, vec![ChunkStep::BeginThenWaitThenCommit]).await);
    case.enqueue().await;
    let mut tasks = JoinSet::new();
    let flushing = Arc::clone(&case);
    tasks.spawn(async move { flushing.flush().await });
    timeout(Duration::from_secs(5), case.writer.entered().notified())
        .await
        .unwrap();

    case.shutdown.cancel();
    let early = tasks.join_next().await.unwrap().unwrap();
    assert!(matches!(early, Err(TagFlushError::Cancelled)));

    case.writer.release().notify_one();
    case.queue.settle_interrupted_commits().await;

    let mut connection = raw(&fixture).await;
    assert_eq!(
        length(&mut connection, REDIS_DAILY_TAG_BUFFER_KEY).await,
        0,
        "a committed cohort is never sent again"
    );
    assert_eq!(
        length(&mut connection, &lease_key(DAILY_TAG_SPEND_JOB_NAME)).await,
        0
    );
}

#[tokio::test]
async fn an_empty_tag_aggregate_commits_nothing_and_pops_the_record() {
    let fixture = RedisFixture::start().await.unwrap();
    let case = TagCase::new(&fixture, Vec::new()).await;
    let mut connection = raw(&fixture).await;
    push(
        &mut connection,
        REDIS_DAILY_TAG_BUFFER_KEY,
        &["{}".to_owned()],
    )
    .await;

    case.flush().await.unwrap();

    assert!(case.events.entries().is_empty());
    assert_eq!(length(&mut connection, REDIS_DAILY_TAG_BUFFER_KEY).await, 0);
    assert_eq!(
        length(&mut connection, &lease_key(DAILY_TAG_SPEND_JOB_NAME)).await,
        0
    );
}

#[tokio::test]
async fn the_adapters_retry_count_reaches_the_tag_commit() {
    let fixture = RedisFixture::start().await.unwrap();
    let case = TagCase::new(&fixture, vec![ChunkStep::Fail(SpendFailure::NotSent); 4]).await;
    case.enqueue().await;

    case.flush().await.unwrap();

    let attempts = case
        .events
        .entries()
        .iter()
        .filter(|event| event.starts_with("daily Tag"))
        .count();
    assert_eq!(attempts, 4, "three retries make four attempts");
    assert_eq!(case.pause.delays().len(), 3);
    let mut connection = raw(&fixture).await;
    assert_eq!(
        length(&mut connection, REDIS_DAILY_TAG_BUFFER_KEY).await,
        1,
        "the whole map goes back after an exhausted retryable failure"
    );
}

#[tokio::test]
async fn a_shutdown_during_a_memory_tag_commit_propagates_as_a_cancellation() {
    let queue = Arc::new(daily_queue(100, 1000, 100));
    queue
        .add_update(daily_row("day-1", "1.5", 1))
        .await
        .unwrap();
    let events = Arc::new(Events::default());
    let writer = Arc::new(FakeDailyWriter::new(
        Arc::clone(&events),
        vec![ChunkStep::WaitThenCommit],
    ));
    let pause = Arc::new(CountingPause::default());
    let parts = parts_with_pause(Arc::clone(&writer), Arc::clone(&pause));
    let commits = TaskTracker::new();
    let shutdown = CancellationToken::new();
    let flusher = MemoryTagFlush {
        queue: &queue,
        parts: &parts,
        commits: &commits,
        shutdown: &shutdown,
    };
    let interrupt = async {
        writer.entered().notified().await;
        shutdown.cancel();
        yield_now().await;
        writer.release().notify_one();
    };

    let (outcome, ()) = tokio::join!(flusher.commit_daily_tag_spend(SPEND_JOB_RETRIES), interrupt);

    let error = outcome.unwrap_err();
    assert!(matches!(error, DailyFlushError::Cancelled { .. }));
    assert!(
        error.is_cancelled(),
        "the adapter must let a shutdown cancellation escape"
    );
}

#[test]
fn a_queued_daily_tag_failure_is_not_a_shutdown_cancellation() {
    let error = DailyFlushError::Queue {
        entity: DailySpendEntity::Tag,
        source: SpendQueueError::CostOverflow,
    };
    assert!(
        !error.is_cancelled(),
        "only a commit the shutdown cancelled counts as one"
    );
}

#[tokio::test]
async fn a_tracked_commit_cancelled_by_its_phase_leaves_nothing_to_restore() {
    let lists = Arc::new(UnreachableLists::default());
    let publisher = publisher_over(&lists);
    let settled = Ok((daily_row("day-1", "1.5", 1), Err(FakeDailyError::Cancelled)));

    super::restore_interrupted_tag::<FakeDailyWriter>(&publisher, settled).await;

    assert_eq!(
        lists.push_attempts(),
        0,
        "a commit the phase cancelled kept every row it had left"
    );
}
