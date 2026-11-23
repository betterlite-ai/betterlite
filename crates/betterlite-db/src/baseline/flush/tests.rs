#![cfg(test)]

use std::future::{Future, pending};
use std::ops::Range;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::FutureExt as _;
use rstest::rstest;
use tokio::sync::Notify;
use tokio::task::yield_now;
use tokio::time::{Instant, timeout};

use super::claim::claim_dirty;
use super::flush_baseline_accounting;
use super::queue::{BaselineQueue, QueueError};
use crate::baseline::publication::model::PublicationStatus;
use crate::baseline::records::BaselineAccountingRecord;
use crate::baseline::store::error::StoreError;
use crate::baseline::store::tests::support::{
    SCOPE, comparison, harness, observation, record, seed_logs, store,
};
use crate::baseline::store::{AppendResult, BaselineAccounting, ProjectResult};
use crate::database_replica::pinned::WriterPinnedClient;

const DIRTY_ROWS: &str = r#"INSERT INTO "LiteLLM_AutoRouterBaselineComparison" (scope, api_key, session_id, router_name, initial_equivalent, revision, published_revision, retired, updated_at)
SELECT 'null-' || lpad(g::text, 2, '0'), 'key', 'session', 'router', TRUE, 1, 0, FALSE, TIMESTAMP '2026-01-02' + g * INTERVAL '1 minute' FROM generate_series(1, 33) AS g;
INSERT INTO "LiteLLM_AutoRouterBaselineComparison" (scope, api_key, session_id, router_name, initial_equivalent, revision, published_revision, attempted_at, retired, updated_at) VALUES
('stale', 'key', 'session', 'router', TRUE, 1, 0, LOCALTIMESTAMP - INTERVAL '31 seconds', FALSE, '2026-01-01'),
('recent', 'key', 'session', 'router', TRUE, 1, 0, LOCALTIMESTAMP, FALSE, '2026-01-01'),
('clean', 'key', 'session', 'router', TRUE, 1, 1, NULL, FALSE, '2026-01-01'),
('retired', 'key', 'session', 'router', TRUE, 1, 0, NULL, TRUE, '2026-01-01')"#;
const LOCKED_ROW: &str = r#"INSERT INTO "LiteLLM_AutoRouterBaselineComparison" (scope, api_key, session_id, router_name, initial_equivalent, revision, published_revision) VALUES ('locked', 'key', 'session', 'router', TRUE, 1, 0), ('free', 'key', 'session', 'router', TRUE, 1, 0)"#;
const LOCK_ROW: &str = r#"BEGIN; SELECT 1 FROM "LiteLLM_AutoRouterBaselineComparison" WHERE scope = 'locked' FOR UPDATE"#;

// What the fake never finishes, so a test can reach the flush deadline or cancel it mid-pass.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hang {
    Nothing,
    Appends,
    Projections,
}

struct FakeAccounting {
    unavailable: Vec<String>,
    hang: Hang,
    claimed: usize,
    claim_fails: bool,
    projected: ProjectResult,
    in_flight: AtomicUsize,
    peak: AtomicUsize,
    calls: Mutex<Vec<String>>,
}

fn fake(unavailable: &[&str], claimed: usize, projected: ProjectResult) -> FakeAccounting {
    FakeAccounting {
        unavailable: unavailable.iter().copied().map(str::to_owned).collect(),
        hang: Hang::Nothing,
        claimed,
        claim_fails: false,
        projected,
        in_flight: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        calls: Mutex::new(Vec::new()),
    }
}

impl BaselineAccounting for FakeAccounting {
    fn append(
        &self,
        record: &BaselineAccountingRecord,
    ) -> impl Future<Output = AppendResult> + Send {
        async move {
            let request_id = record.observation.request_id.clone();
            self.calls
                .lock()
                .unwrap()
                .push(format!("append:{request_id}"));
            let running = self
                .in_flight
                .fetch_add(1, Ordering::SeqCst)
                .saturating_add(1);
            self.peak.fetch_max(running, Ordering::SeqCst);
            if self.hang == Hang::Appends {
                pending::<()>().await;
            }
            yield_now().await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            if self.unavailable.contains(&request_id) {
                AppendResult::Unavailable
            } else {
                AppendResult::Recorded
            }
        }
    }

    fn project(&self, scope: &str) -> impl Future<Output = ProjectResult> + Send {
        async move {
            self.calls.lock().unwrap().push(format!("project:{scope}"));
            if self.hang == Hang::Projections {
                pending::<()>().await;
            }
            self.projected
        }
    }

    fn claim_dirty(&self) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send {
        async move {
            self.calls.lock().unwrap().push("claim".to_owned());
            if self.claim_fails {
                return Err(StoreError::Revision);
            }
            Ok((0..self.claimed)
                .map(|index| format!("scope-{index}"))
                .collect())
        }
    }
}

impl FakeAccounting {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

async fn queue_of(capacity: usize, count: u32) -> BaselineQueue {
    let queue = BaselineQueue::new(capacity).unwrap();
    for index in 0..count {
        let value = record(&format!("req-{index}"), f64::from(index));
        queue.push(value).await.unwrap();
    }
    queue
}

fn queued(queue: &BaselineQueue) -> Vec<String> {
    let lease = queue.lease(usize::MAX);
    lease
        .records
        .iter()
        .map(|entry| entry.record.observation.request_id.clone())
        .collect()
}

fn ids(range: Range<u32>) -> Vec<String> {
    range.map(|index| format!("req-{index}")).collect()
}

#[tokio::test]
async fn unavailable_records_go_back_behind_the_records_still_pending() {
    let queue = queue_of(64, 34).await;
    let accounting = fake(&["req-1", "req-5"], 0, ProjectResult::Unchanged);
    flush_baseline_accounting(&accounting, &queue, &Notify::new()).await;
    assert_eq!(queued(&queue), ["req-32", "req-33", "req-1", "req-5"]);
    let mut expected: Vec<String> = ids(0..32)
        .into_iter()
        .map(|id| format!("append:{id}"))
        .collect();
    expected.push("claim".to_owned());
    assert_eq!(accounting.calls(), expected);
}

#[rstest]
#[case::more_waiting_and_some_completed(33, false, true)]
#[case::more_waiting_but_none_completed(33, true, false)]
#[case::nothing_more_waiting(32, false, false)]
#[tokio::test]
async fn another_flush_is_requested_only_when_more_waited_and_a_record_completed(
    #[case] count: u32,
    #[case] all_fail: bool,
    #[case] requested: bool,
) {
    let queue = queue_of(64, count).await;
    let failures = if all_fail { ids(0..32) } else { Vec::new() };
    let mut accounting = fake(&[], 0, ProjectResult::Unchanged);
    accounting.unavailable = failures;
    let notifier = Notify::new();
    flush_baseline_accounting(&accounting, &queue, &notifier).await;
    assert_eq!(notifier.notified().now_or_never().is_some(), requested);
}

#[tokio::test(start_paused = true)]
async fn a_timed_out_flush_requeues_the_whole_batch_after_the_pending_records() {
    let queue = queue_of(64, 33).await;
    let mut accounting = fake(&[], 0, ProjectResult::Unchanged);
    accounting.hang = Hang::Appends;
    let started = Instant::now();
    flush_baseline_accounting(&accounting, &queue, &Notify::new()).await;
    assert!((Duration::from_secs(5)..Duration::from_secs(6)).contains(&started.elapsed()));
    let mut expected = vec!["req-32".to_owned()];
    expected.extend(ids(0..32));
    assert_eq!(queued(&queue), expected);
    assert!(!accounting.calls().contains(&"claim".to_owned()));
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_flush_requeues_the_whole_batch_after_the_pending_records() {
    let queue = queue_of(64, 33).await;
    let mut accounting = fake(&[], 0, ProjectResult::Unchanged);
    accounting.hang = Hang::Appends;
    let notifier = Notify::new();
    let mut flush = Box::pin(flush_baseline_accounting(&accounting, &queue, &notifier));
    timeout(Duration::from_millis(10), &mut flush)
        .await
        .unwrap_err();
    assert_eq!(queued(&queue), ["req-32"]);
    drop(flush);
    let mut expected = vec!["req-32".to_owned()];
    expected.extend(ids(0..32));
    assert_eq!(queued(&queue), expected);
}

#[tokio::test]
async fn appends_run_at_most_four_at_a_time() {
    let queue = queue_of(64, 32).await;
    let accounting = fake(&[], 0, ProjectResult::Unchanged);
    flush_baseline_accounting(&accounting, &queue, &Notify::new()).await;
    assert_eq!(accounting.peak.load(Ordering::SeqCst), 4);
    assert!(queued(&queue).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_requeued_record_keeps_its_slot_so_a_full_queue_neither_deadlocks_nor_drops_it() {
    let queue = queue_of(2, 2).await;
    let mut hanging = fake(&[], 0, ProjectResult::Unchanged);
    hanging.hang = Hang::Appends;
    flush_baseline_accounting(&hanging, &queue, &Notify::new()).await;
    assert_eq!(queued(&queue), ["req-0", "req-1"]);
    let blocked = timeout(Duration::from_secs(1), queue.push(record("req-2", 2.0_f64))).await;
    blocked.unwrap_err();
    let recording = fake(&["req-1"], 0, ProjectResult::Unchanged);
    flush_baseline_accounting(&recording, &queue, &Notify::new()).await;
    assert_eq!(queued(&queue), ["req-1"]);
    let admitted = timeout(Duration::from_secs(1), queue.push(record("req-2", 2.0_f64))).await;
    admitted.unwrap().unwrap();
    assert_eq!(queued(&queue), ["req-1", "req-2"]);
}

#[rstest]
#[case::full_claim_with_a_publication(32, ProjectResult::Published, true)]
#[case::partial_claim(31, ProjectResult::Published, false)]
#[case::full_claim_without_a_publication(32, ProjectResult::Unavailable, false)]
#[tokio::test]
async fn a_full_claim_with_a_published_projection_requests_another_flush(
    #[case] claimed: usize,
    #[case] projected: ProjectResult,
    #[case] requested: bool,
) {
    let queue = queue_of(4, 0).await;
    let accounting = fake(&[], claimed, projected);
    let notifier = Notify::new();
    flush_baseline_accounting(&accounting, &queue, &notifier).await;
    assert_eq!(notifier.notified().now_or_never().is_some(), requested);
    let projections = accounting
        .calls()
        .iter()
        .filter(|call| call.starts_with("project:"))
        .count();
    assert_eq!(projections, claimed);
    assert_eq!(accounting.peak.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_failed_claim_projects_nothing() {
    let queue = queue_of(4, 0).await;
    let mut accounting = fake(&[], 32, ProjectResult::Published);
    accounting.claim_fails = true;
    let notifier = Notify::new();
    flush_baseline_accounting(&accounting, &queue, &notifier).await;
    assert_eq!(accounting.calls(), ["claim"]);
    assert!(notifier.notified().now_or_never().is_none());
}

#[tokio::test(start_paused = true)]
async fn slow_projections_stop_at_the_flush_deadline_without_requesting_another_flush() {
    let queue = queue_of(4, 0).await;
    let mut accounting = fake(&[], 32, ProjectResult::Published);
    accounting.hang = Hang::Projections;
    let notifier = Notify::new();
    let started = Instant::now();
    flush_baseline_accounting(&accounting, &queue, &notifier).await;
    assert!((Duration::from_secs(5)..Duration::from_secs(6)).contains(&started.elapsed()));
    assert!(notifier.notified().now_or_never().is_none());
    let projections = accounting.calls().len().saturating_sub(1);
    assert_eq!(projections, 4);
}

#[tokio::test]
async fn the_queue_needs_a_capacity_and_wakes_waiting_producers_when_closed() {
    assert!(matches!(
        BaselineQueue::new(0).err(),
        Some(QueueError::ZeroCapacity)
    ));
    assert!(matches!(
        BaselineQueue::new(usize::MAX).err(),
        Some(QueueError::CapacityTooLarge)
    ));
    let queue = queue_of(1, 1).await;
    let mut waiting = Box::pin(queue.push(record("req-1", 1.0_f64)));
    timeout(Duration::from_millis(10), &mut waiting)
        .await
        .unwrap_err();
    assert_eq!(queue.close(), 1);
    assert!(matches!(waiting.await, Err(QueueError::Closed)));
    assert_eq!(queued(&queue), ["req-0"]);
}

#[tokio::test]
async fn a_flush_against_postgres_acknowledges_the_record_and_projects_its_scope() {
    let harness = harness().await;
    let client = harness.pool.get().await.unwrap();
    seed_logs(&client, &["req-0"]).await;
    let queue = queue_of(4, 1).await;
    let accounting = store(&harness.writer);
    flush_baseline_accounting(&accounting, &queue, &Notify::new()).await;
    assert!(queued(&queue).is_empty());
    let state = comparison(&client, SCOPE).await;
    assert_eq!((state.revision, state.published), (1, 1));
    assert!(!state.unattempted);
    let published = observation(&client, "req-0")
        .await
        .unwrap()
        .publication
        .unwrap();
    assert_eq!(published.status, PublicationStatus::Estimated);
    drop(client);
    harness.close().await;
}

#[tokio::test]
async fn a_claim_takes_unattempted_scopes_first_and_waits_out_the_retry_interval() {
    let harness = harness().await;
    let client = harness.pool.get().await.unwrap();
    client.batch_execute(DIRTY_ROWS).await.unwrap();
    let database = WriterPinnedClient::Writer(&harness.writer);
    let mut first = claim_dirty(database).await.unwrap();
    first.sort();
    let expected: Vec<String> = (1_u32..=32_u32)
        .map(|index| format!("null-{index:02}"))
        .collect();
    assert_eq!(first, expected);
    let mut second = claim_dirty(database).await.unwrap();
    second.sort();
    assert_eq!(second, ["null-33", "stale"]);
    assert!(claim_dirty(database).await.unwrap().is_empty());
    drop(client);
    harness.close().await;
}

#[tokio::test]
async fn a_claim_skips_a_scope_another_transaction_holds() {
    let harness = harness().await;
    let holder = harness.pool.get().await.unwrap();
    holder.batch_execute(LOCKED_ROW).await.unwrap();
    holder.batch_execute(LOCK_ROW).await.unwrap();
    let database = WriterPinnedClient::Writer(&harness.writer);
    assert_eq!(claim_dirty(database).await.unwrap(), ["free"]);
    holder.batch_execute("COMMIT").await.unwrap();
    assert_eq!(claim_dirty(database).await.unwrap(), ["locked"]);
    drop(holder);
    harness.close().await;
}
