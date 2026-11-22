#![cfg(test)]

use std::collections::VecDeque;
use std::iter::repeat_n;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::redis_spend::buffer::RedisUpdateBuffer;
use crate::redis_spend::events::NoRedisBufferEventSink;
use crate::redis_spend::log_row::SpendLogRow;
use crate::spend_batches::StatementLimits;
use crate::spend_log_storage::rows::{MAX_LOGS_PER_INTERVAL, QueuePosition, SpendLogQueue};
use crate::spend_log_storage::tests::support::{
    NoClient, NoHttpFailure, RecordingPause, ScriptedFailure, SteppingClock, ids, ordinary_row,
    spend_row,
};
use crate::spend_log_storage::{SpendLogFuture, SpendLogsWriter};

use super::{
    DEFAULT_RETRIES, FlushError, FlushPolicy, JSON_CONTENT_TYPE, NON_BLOCKING_PREFIX,
    SpendLogFailureEvent, SpendLogFailureSink, SpendLogFlush, SpendUpdateClient,
    SpendUpdateRequest, UPDATE_SPEND_CALL_TYPE,
};

const FULL_LIMITS: StatementLimits = StatementLimits {
    max_bytes: usize::MAX,
    max_rows: 10_000,
};

#[derive(Default)]
struct ScriptedWriter {
    failures: Mutex<VecDeque<ScriptedFailure>>,
    attempts: Mutex<Vec<usize>>,
}

impl ScriptedWriter {
    fn failing(failure: ScriptedFailure, times: usize) -> Self {
        Self {
            failures: Mutex::new(repeat_n(failure, times).collect()),
            attempts: Mutex::new(Vec::new()),
        }
    }

    fn attempt(&self, rows: &[SpendLogRow]) -> Result<usize, ScriptedFailure> {
        self.attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(rows.len());
        let pending = self
            .failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(failure) = pending else {
            return Ok(rows.len());
        };
        Err(failure)
    }

    fn attempted(&self) -> Vec<usize> {
        self.attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn attempt_count(&self) -> usize {
        self.attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl SpendLogsWriter for ScriptedWriter {
    type Error = ScriptedFailure;

    fn insert_rows<'call>(
        &'call self,
        rows: &'call [SpendLogRow],
    ) -> SpendLogFuture<'call, Result<usize, ScriptedFailure>> {
        Box::pin(async move { self.attempt(rows) })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Posted {
    url: String,
    body: String,
    content_type: &'static str,
}

struct RecordingClient {
    status: u16,
    fail: bool,
    posts: Mutex<Vec<Posted>>,
}

impl SpendUpdateClient for RecordingClient {
    type Error = NoHttpFailure;

    fn post<'call>(
        &'call self,
        request: SpendUpdateRequest<'call>,
    ) -> SpendLogFuture<'call, Result<u16, NoHttpFailure>> {
        Box::pin(async move {
            if self.fail {
                return Err(NoHttpFailure);
            }
            self.posts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(Posted {
                    url: request.url.to_owned(),
                    body: request.body.to_owned(),
                    content_type: request.content_type,
                });
            Ok(self.status)
        })
    }
}

#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<SpendLogFailureEvent>>,
}

impl SpendLogFailureSink for RecordingSink {
    fn emit(&self, event: SpendLogFailureEvent) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

struct Harness {
    writer: ScriptedWriter,
    client: RecordingClient,
    queue: SpendLogQueue,
    redis: RedisUpdateBuffer,
    pause: RecordingPause,
    clock: SteppingClock,
    failures: RecordingSink,
    limits: StatementLimits,
}

impl Harness {
    fn new() -> Self {
        Self {
            writer: ScriptedWriter::default(),
            client: RecordingClient {
                status: 200,
                fail: false,
                posts: Mutex::new(Vec::new()),
            },
            queue: SpendLogQueue::new(usize::MAX),
            redis: RedisUpdateBuffer {
                lists: None,
                enabled: false,
                dequeue_count: 10,
                events: Arc::new(NoRedisBufferEventSink),
            },
            pause: RecordingPause::default(),
            clock: SteppingClock {
                start: Instant::now(),
                steps: AtomicU32::new(0),
            },
            failures: RecordingSink::default(),
            limits: FULL_LIMITS,
        }
    }

    fn posts(&self) -> Vec<Posted> {
        self.client
            .posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn events(&self) -> Vec<SpendLogFailureEvent> {
        self.failures
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    async fn run(
        &self,
        logs: Option<&[SpendLogRow]>,
        url: Option<&str>,
        retries: u32,
    ) -> Result<(), FlushError<ScriptedFailure, NoHttpFailure>> {
        let flush = SpendLogFlush {
            writer: &self.writer,
            external: Some(&self.client),
            queue: &self.queue,
            redis: &self.redis,
            failures: &self.failures,
            pause: &self.pause,
            clock: &self.clock,
            policy: FlushPolicy {
                retries,
                limits: self.limits,
                spend_logs_url: url.map(str::to_owned),
            },
        };
        flush.run(logs).await
    }
}

#[tokio::test]
async fn an_external_writer_posts_the_array_to_spend_update() {
    let harness = Harness::new();
    let rows = [spend_row("batch-1")];
    harness
        .run(
            Some(&rows),
            Some("http://spend-writer.internal"),
            DEFAULT_RETRIES,
        )
        .await
        .expect("the post lands");
    let posts = harness.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].url, "http://spend-writer.internal/spend/update");
    assert_eq!(posts[0].content_type, JSON_CONTENT_TYPE);
    assert_eq!(
        posts[0].body,
        serde_json::to_string(&rows).expect("the posted body")
    );
    assert_eq!(harness.writer.attempt_count(), 0);
}

#[tokio::test]
async fn a_trailing_slash_is_not_doubled_in_the_spend_update_url() {
    let harness = Harness::new();
    let rows = [spend_row("batch-1")];
    harness
        .run(
            Some(&rows),
            Some("http://spend-writer.internal/"),
            DEFAULT_RETRIES,
        )
        .await
        .expect("the post lands");
    assert_eq!(
        harness.posts()[0].url,
        "http://spend-writer.internal/spend/update"
    );
}

#[tokio::test]
async fn a_non_200_response_still_completes_the_pass() {
    let mut harness = Harness::new();
    harness.client.status = 500;
    let rows = [ordinary_row("request-1")];
    harness
        .run(
            Some(&rows),
            Some("http://spend-writer.internal"),
            DEFAULT_RETRIES,
        )
        .await
        .expect("a non-200 still completes the pass");
    assert_eq!(harness.posts().len(), 1);
    assert_eq!(harness.writer.attempt_count(), 0);
    assert!(harness.events().is_empty());
}

#[tokio::test]
async fn an_external_transport_error_propagates_without_a_retry_or_a_requeue() {
    let mut harness = Harness::new();
    harness.client.fail = true;
    harness
        .queue
        .enqueue(&[ordinary_row("request-1")], QueuePosition::Tail);
    let error = harness
        .run(None, Some("http://spend-writer.internal"), DEFAULT_RETRIES)
        .await
        .expect_err("the transport failure propagates");
    assert!(matches!(error, FlushError::External(_)));
    assert_eq!(harness.posts().len(), 0);
    assert!(harness.queue.is_empty());
    assert!(harness.pause.delays().is_empty());
    assert_eq!(harness.events().len(), 1);
}

#[tokio::test]
async fn a_transient_error_is_retried_after_one_then_two_seconds() {
    let harness = Harness {
        writer: ScriptedWriter::failing(ScriptedFailure::OUTAGE, 2),
        ..Harness::new()
    };
    let rows = [ordinary_row("request-1"), ordinary_row("request-2")];
    harness
        .run(Some(&rows), None, DEFAULT_RETRIES)
        .await
        .expect("the third attempt lands");
    assert_eq!(harness.writer.attempt_count(), 3);
    assert_eq!(
        harness.pause.delays(),
        [Duration::from_secs(1), Duration::from_secs(2)]
    );
    assert!(harness.events().is_empty());
}

#[tokio::test]
async fn a_transient_error_that_never_settles_is_requeued_after_the_last_attempt() {
    let harness = Harness {
        writer: ScriptedWriter::failing(ScriptedFailure::OUTAGE, 4),
        ..Harness::new()
    };
    let rows = [ordinary_row("request-1"), ordinary_row("request-2")];
    let error = harness
        .run(Some(&rows), None, DEFAULT_RETRIES)
        .await
        .expect_err("the last attempt propagates");
    assert!(matches!(error, FlushError::Store(_)));
    assert_eq!(harness.writer.attempt_count(), 4);
    assert_eq!(
        harness.pause.delays(),
        [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4)
        ]
    );
    assert_eq!(ids(&harness.queue.dequeue(10)), ["request-1", "request-2"]);
    let events = harness.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].call_type, UPDATE_SPEND_CALL_TYPE);
    assert!(events[0].diagnostic.starts_with(NON_BLOCKING_PREFIX));
    assert_eq!(events[0].duration, Duration::from_secs(5));
}

#[tokio::test]
async fn a_database_error_that_is_not_transient_is_requeued_once() {
    let harness = Harness {
        writer: ScriptedWriter::failing(ScriptedFailure::DATABASE_ERROR, 1),
        ..Harness::new()
    };
    let rows = [ordinary_row("request-1")];
    let error = harness
        .run(Some(&rows), None, DEFAULT_RETRIES)
        .await
        .expect_err("the database error propagates");
    assert!(matches!(error, FlushError::Store(_)));
    assert_eq!(harness.writer.attempt_count(), 1);
    assert!(harness.pause.delays().is_empty());
    assert_eq!(ids(&harness.queue.dequeue(10)), ["request-1"]);
    assert_eq!(harness.events().len(), 1);
}

#[tokio::test]
async fn an_unrelated_error_propagates_without_a_database_replay() {
    let harness = Harness {
        writer: ScriptedWriter::failing(ScriptedFailure::NON_DATABASE, 1),
        ..Harness::new()
    };
    let rows = [ordinary_row("request-1")];
    let error = harness
        .run(Some(&rows), None, DEFAULT_RETRIES)
        .await
        .expect_err("the error propagates");
    assert!(matches!(error, FlushError::Store(_)));
    assert_eq!(harness.writer.attempt_count(), 1);
    assert!(harness.pause.delays().is_empty());
    assert!(harness.queue.is_empty());
    assert_eq!(harness.events().len(), 1);
}

#[tokio::test]
async fn a_pass_writes_groups_of_a_thousand_rows() {
    let harness = Harness::new();
    let rows: Vec<SpendLogRow> = (0..2500_u32)
        .map(|index| ordinary_row(&format!("request-{index}")))
        .collect();
    harness
        .run(Some(&rows), None, DEFAULT_RETRIES)
        .await
        .expect("the groups land");
    assert_eq!(harness.writer.attempted(), vec![1000_usize, 1000, 500]);
    assert!(harness.events().is_empty());
}

#[tokio::test]
async fn a_group_is_split_by_the_statement_row_limit() {
    let mut harness = Harness::new();
    harness.limits = StatementLimits {
        max_bytes: usize::MAX,
        max_rows: 3,
    };
    let rows: Vec<SpendLogRow> = (0..5_u32)
        .map(|index| ordinary_row(&format!("request-{index}")))
        .collect();
    harness
        .run(Some(&rows), None, DEFAULT_RETRIES)
        .await
        .expect("the statements land");
    assert_eq!(harness.writer.attempted(), vec![3_usize, 2]);
}

#[tokio::test]
async fn a_pass_pops_at_most_ten_thousand_rows() {
    let harness = Harness::new();
    let rows: Vec<SpendLogRow> = (0..10_001_u32)
        .map(|index| ordinary_row(&format!("request-{index}")))
        .collect();
    harness.queue.enqueue(&rows, QueuePosition::Tail);
    harness
        .run(None, None, DEFAULT_RETRIES)
        .await
        .expect("the pass lands");
    assert_eq!(
        harness.writer.attempted().iter().sum::<usize>(),
        MAX_LOGS_PER_INTERVAL
    );
    assert_eq!(harness.queue.len(), 1);
    assert_eq!(harness.writer.attempted(), vec![1000_usize; 10]);
}

#[tokio::test]
async fn an_empty_queue_makes_no_statement() {
    let harness = Harness::new();
    harness
        .run(None, None, DEFAULT_RETRIES)
        .await
        .expect("nothing to write");
    assert_eq!(harness.writer.attempt_count(), 0);
    assert!(harness.events().is_empty());
}

#[tokio::test]
async fn a_caller_owned_batch_leaves_the_queue_alone() {
    let harness = Harness::new();
    harness
        .queue
        .enqueue(&[ordinary_row("queued")], QueuePosition::Tail);
    let rows = [ordinary_row("owned")];
    harness
        .run(Some(&rows), None, DEFAULT_RETRIES)
        .await
        .expect("the owned batch lands");
    assert_eq!(ids(&harness.queue.dequeue(10)), ["queued"]);
}

#[tokio::test]
async fn a_url_without_a_client_still_writes_to_the_database() {
    let writer = ScriptedWriter::default();
    let queue = SpendLogQueue::new(usize::MAX);
    let redis = RedisUpdateBuffer {
        lists: None,
        enabled: false,
        dequeue_count: 10,
        events: Arc::new(NoRedisBufferEventSink),
    };
    let pause = RecordingPause::default();
    let clock = SteppingClock {
        start: Instant::now(),
        steps: AtomicU32::new(0),
    };
    let failures = RecordingSink::default();
    let flush = SpendLogFlush {
        writer: &writer,
        external: None::<&NoClient>,
        queue: &queue,
        redis: &redis,
        failures: &failures,
        pause: &pause,
        clock: &clock,
        policy: FlushPolicy {
            retries: 0,
            limits: FULL_LIMITS,
            spend_logs_url: Some("http://spend-writer.internal".to_owned()),
        },
    };
    let rows = [ordinary_row("request-1")];
    flush
        .run(Some(&rows))
        .await
        .expect("the database branch runs");
    assert_eq!(writer.attempted(), vec![1_usize]);
    assert!(queue.is_empty());
}
