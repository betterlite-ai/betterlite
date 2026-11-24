#![cfg(test)]

use std::future::ready;
use std::sync::Arc;
use std::time::Instant;

use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::gateway_request_tracking::test_support::captured_logs;
use crate::spend_commit::policy::{SpendFailure, SpendRetryPolicy};
use crate::spend_flush::daily::DailyCommitParts;
use crate::spend_flush::fakes::tests::{
    ChunkStep, Events, FakeCache, FakeDailyWriter, FakeDiscovery, FakeEntityWriter, FakeService,
    FakeWindow, FixedClock, FixedJitter,
};
use crate::spend_flush::memory::{DailySpendQueues, MemorySpendFlush};
use crate::spend_flush::redis::fakes::tests::{
    CountingPause, Queues, parts_with_pause, recording_pass, spend_queues,
};
use crate::spend_flush::redis::jobs::{
    DailyTagSpendError, DailyTagSpendFlush, SPEND_JOB_RETRIES, SpendUpdateJob, UpdateSpendJobError,
};
use crate::spend_flush::redis::leader::dispatch::{
    SpendUpdateError, SpendUpdateTransactionHandler,
};
use crate::spend_flush::redis::tags::{DailyTagCommit, MemoryTagFlush, TagCommitFailure};
use crate::spend_flush::service::ServiceFuture;
use crate::spend_log_storage::rows::writer::NoSpendLogFailureSink;
use crate::spend_queues::entity::EntityType;

#[tokio::test]
async fn update_spend_runs_the_balance_dispatch_then_the_logs_job() {
    let events = Arc::new(Events::default());
    let redis = recording_pass("redis", Arc::clone(&events), false);
    let memory = recording_pass("memory", Arc::clone(&events), false);
    let service = FakeService::new(Arc::clone(&events), vec![1]);
    let job = SpendUpdateJob {
        balance: SpendUpdateTransactionHandler::select(true, &redis, &memory),
        logs: &service,
    };

    job.update_spend().await.unwrap();

    assert_eq!(
        events.entries(),
        [
            "balance redis 3".to_owned(),
            "recover".to_owned(),
            "census 1".to_owned(),
            "update Succeed".to_owned(),
        ]
    );
}

#[tokio::test]
async fn update_spend_skips_the_logs_job_when_the_census_is_zero() {
    let events = Arc::new(Events::default());
    let redis = recording_pass("redis", Arc::clone(&events), false);
    let memory = recording_pass("memory", Arc::clone(&events), false);
    let service = FakeService::new(Arc::clone(&events), vec![0]);
    let job = SpendUpdateJob {
        balance: SpendUpdateTransactionHandler::select(false, &redis, &memory),
        logs: &service,
    };

    job.update_spend().await.unwrap();

    assert_eq!(
        events.entries(),
        [
            "balance memory 3".to_owned(),
            "recover".to_owned(),
            "census 0".to_owned(),
        ]
    );
}

#[tokio::test]
async fn a_balance_failure_stops_before_the_logs_job() {
    let events = Arc::new(Events::default());
    let redis = recording_pass("redis", Arc::clone(&events), true);
    let memory = recording_pass("memory", Arc::clone(&events), false);
    let service = FakeService::new(Arc::clone(&events), vec![1]);
    let job = SpendUpdateJob {
        balance: SpendUpdateTransactionHandler::select(true, &redis, &memory),
        logs: &service,
    };

    let error = job.update_spend().await.unwrap_err();

    assert!(matches!(error, UpdateSpendJobError::Balance(_)));
    assert_eq!(events.entries(), ["balance redis 3".to_owned()]);
}

#[derive(Debug, thiserror::Error)]
#[error("the tag commit exploded")]
struct TagFailure {
    #[source]
    cause: TagCause,
    cancelled: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("the inner tag cause")]
struct TagCause;

impl TagCommitFailure for TagFailure {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}

#[derive(Clone, Copy)]
enum TagOutcome {
    Succeed,
    Cancel,
    Fail,
}

// One tag adapter branch that records the retry count it was handed and settles on script.
struct TagRecorder {
    name: &'static str,
    events: Arc<Events>,
    outcome: TagOutcome,
}

impl DailyTagCommit for TagRecorder {
    type Error = TagFailure;

    fn commit_daily_tag_spend(&self, retries: u32) -> ServiceFuture<'_, Result<(), TagFailure>> {
        self.events.push(format!("tag {} {retries}", self.name));
        let outcome = match self.outcome {
            TagOutcome::Succeed => Ok(()),
            TagOutcome::Cancel => Err(TagFailure {
                cause: TagCause,
                cancelled: true,
            }),
            TagOutcome::Fail => Err(TagFailure {
                cause: TagCause,
                cancelled: false,
            }),
        };
        Box::pin(ready(outcome))
    }
}

fn tag_recorder(name: &'static str, events: Arc<Events>, outcome: TagOutcome) -> TagRecorder {
    TagRecorder {
        name,
        events,
        outcome,
    }
}

#[tokio::test]
async fn the_tag_adapter_logs_an_ordinary_failure_without_its_source() {
    let events = Arc::new(Events::default());
    let redis = tag_recorder("redis", Arc::clone(&events), TagOutcome::Fail);
    let memory = tag_recorder("memory", Arc::clone(&events), TagOutcome::Succeed);
    let (logs, _guard) = captured_logs();

    DailyTagSpendFlush::select(true, &redis, &memory)
        .update_daily_tag_spend()
        .await
        .unwrap();

    assert_eq!(events.entries(), ["tag redis 3".to_owned()]);
    let text = logs.text();
    assert!(
        text.contains("Error updating daily tag spend: the tag commit exploded"),
        "the adapter names the failure: {text}"
    );
    assert!(
        !text.contains("the inner tag cause") && !text.contains("Caused by"),
        "the source chain stays out of the plain log: {text}"
    );
}

#[tokio::test]
async fn a_cancelled_tag_commit_propagates_instead_of_being_logged() {
    let events = Arc::new(Events::default());
    let redis = tag_recorder("redis", Arc::clone(&events), TagOutcome::Cancel);
    let memory = tag_recorder("memory", Arc::clone(&events), TagOutcome::Succeed);
    let (logs, _guard) = captured_logs();

    let error = DailyTagSpendFlush::select(true, &redis, &memory)
        .update_daily_tag_spend()
        .await
        .unwrap_err();

    assert!(error.is_cancelled());
    let DailyTagSpendError::Redis(interrupted) = error else {
        panic!("the redis branch was selected");
    };
    assert!(interrupted.is_cancelled());
    assert!(
        !logs.text().contains("Error updating daily tag spend"),
        "a shutdown cancellation is not an ordinary failure"
    );
    assert_eq!(events.entries(), ["tag redis 3".to_owned()]);
}

#[tokio::test]
async fn the_adapters_retry_count_reaches_the_memory_tag_commit() {
    let events = Arc::new(Events::default());
    let writer = Arc::new(FakeDailyWriter::new(
        Arc::clone(&events),
        vec![ChunkStep::Fail(SpendFailure::NotSent); 4],
    ));
    let pause = Arc::new(CountingPause::default());
    let parts = parts_with_pause(Arc::clone(&writer), Arc::clone(&pause));
    let queues = spend_queues();
    queues.add_daily_user("2026-01-02", "1").await;
    let commits = TaskTracker::new();
    let shutdown = CancellationToken::new();
    let flusher = MemoryTagFlush {
        queue: &queues.daily_user,
        parts: &parts,
        commits: &commits,
        shutdown: &shutdown,
    };
    let redis = tag_recorder("redis", Arc::clone(&events), TagOutcome::Succeed);

    DailyTagSpendFlush::select(false, &redis, &flusher)
        .update_daily_tag_spend()
        .await
        .unwrap();

    let entries = events.entries();
    let attempts = entries
        .iter()
        .filter(|event| event.starts_with("daily Tag"))
        .count();
    assert_eq!(attempts, 4, "three retries make four attempts");
    assert_eq!(pause.delays().len(), 3);
    assert!(
        !entries.iter().any(|event| event.starts_with("tag redis")),
        "the memory branch is the one that ran"
    );
}

// The memory branch's ports and events, owned so a case reads them after the commit and its failed entity writer is in place.
struct MemoryCase {
    queues: Queues,
    events: Arc<Events>,
    entity_writer: FakeEntityWriter,
    cache: FakeCache,
    window: FakeWindow,
    discovery: FakeDiscovery,
    commits: TaskTracker,
    shutdown: CancellationToken,
    clock: FixedClock,
    jitter: FixedJitter,
    pause: Arc<CountingPause>,
    failures: NoSpendLogFailureSink,
    parts: Arc<DailyCommitParts<FakeDailyWriter>>,
}

impl MemoryCase {
    fn new(events: Arc<Events>) -> Self {
        let writer = Arc::new(FakeDailyWriter::new(Arc::clone(&events), Vec::new()));
        let pause = Arc::new(CountingPause::default());
        let parts = parts_with_pause(writer, Arc::clone(&pause));
        let entity_writer = FakeEntityWriter::new(Arc::clone(&events));
        entity_writer.fail_with(SpendFailure::Deadlock);
        Self {
            queues: spend_queues(),
            cache: FakeCache::new(Arc::clone(&events)),
            window: FakeWindow::new(Arc::clone(&events)),
            discovery: FakeDiscovery::new(Arc::clone(&events)),
            commits: TaskTracker::new(),
            shutdown: CancellationToken::new(),
            clock: FixedClock {
                instant: Instant::now(),
            },
            jitter: FixedJitter,
            failures: NoSpendLogFailureSink,
            entity_writer,
            parts,
            pause,
            events,
        }
    }

    // The memory branch's one tick through the source's selector at the job adapter's retry count; true when its failure surfaces as the memory variant.
    async fn commit(&self) -> bool {
        let policy = SpendRetryPolicy {
            retries: 0,
            clock: &self.clock,
            jitter: &self.jitter,
            pause: &*self.pause,
            failures: &self.failures,
        };
        let flush = MemorySpendFlush {
            entity_queue: &self.queues.entity,
            entity_writer: &self.entity_writer,
            entity_cache: &self.cache,
            policy,
            daily: DailySpendQueues {
                user: &self.queues.daily_user,
                team: &self.queues.daily_team,
                org: &self.queues.daily_org,
                end_user: &self.queues.daily_end_user,
                agent: &self.queues.daily_agent,
            },
            daily_commits: &self.commits,
            daily_parts: &self.parts,
            window_queue: &self.queues.window,
            window: &self.window,
            discovery_queue: &self.queues.discovery,
            discovery: &self.discovery,
            shutdown: &self.shutdown,
        };
        let redis = recording_pass("redis", Arc::clone(&self.events), false);
        SpendUpdateTransactionHandler::select(false, &redis, &flush)
            .commit_spend_updates(SPEND_JOB_RETRIES)
            .await
            .is_err_and(|error| matches!(error, SpendUpdateError::Memory(_)))
    }
}

#[tokio::test]
async fn the_memory_branch_retries_the_entity_commit_with_the_adapters_count() {
    let case = MemoryCase::new(Arc::new(Events::default()));
    case.queues
        .add_entity(EntityType::User, "u-memory", "2")
        .await;

    assert!(
        case.commit().await,
        "the exhausted entity commit surfaces as the memory branch's error"
    );

    let entries = case.events.entries();
    let attempts = entries
        .iter()
        .filter(|event| event.starts_with("entity user_list_transactions"))
        .count();
    assert_eq!(
        attempts, 4,
        "the handler's retry count reaches the memory entity commit"
    );
    assert_eq!(case.pause.delays().len(), 3);
    assert!(
        !entries.iter().any(|event| event.starts_with("balance")),
        "the redis branch is not the one that ran"
    );
}
