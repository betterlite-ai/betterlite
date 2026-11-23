#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use rust_decimal::Decimal;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::{DailySpendQueues, MemoryFlushError, MemorySpendFlush, spend_table_transactions};
use crate::daily_spend_sql::entities::{DailySpendEntity, SpendRow};
use crate::daily_spend_sql::values::SpendColumnValue;
use crate::spend_commit::policy::{SpendFailure, SpendRetryPolicy};
use crate::spend_commit::tables::{SpendTable, SpendTableTransactions};
use crate::spend_flush::daily::{DailyCommitParts, DailyFlushError};
use crate::spend_flush::fakes::tests::{
    ChunkStep, Events, FakeCache, FakeCacheError, FakeDailyWriter, FakeDiscovery, FakeEntityError,
    FakeEntityWriter, FakeWindow, FixedClock, FixedJitter, PauseLog,
};
use crate::spend_log_storage::rows::writer::NoSpendLogFailureSink;
use crate::spend_queues::daily::{DailySpendUpdate, DailySpendUpdateQueue};
use crate::spend_queues::entity::{
    EntityType, SpendAmount, SpendUpdateQueue, SpendUpdateQueueItem, SpendUpdateTransactions,
};
use crate::spend_queues::events::NoSpendQueueEventSink;
use crate::tool_discovery::{ToolDiscoveryItem, ToolDiscoveryQueue};
use crate::window_queue::WindowQueueError;
use crate::window_queue::WindowSpendTransaction;
use crate::window_queue::queue::WindowSpendUpdateQueue;

// The whole memory pass over fakes: every queue starts empty and every port succeeds unless a test says otherwise.
struct Harness {
    events: Arc<Events>,
    entity_queue: SpendUpdateQueue,
    entity_writer: FakeEntityWriter,
    cache: FakeCache,
    clock: FixedClock,
    jitter: FixedJitter,
    pause: PauseLog,
    failures: NoSpendLogFailureSink,
    daily: [Arc<DailySpendUpdateQueue>; 5],
    commits: TaskTracker,
    parts: Arc<DailyCommitParts<FakeDailyWriter>>,
    window_queue: WindowSpendUpdateQueue,
    window: FakeWindow,
    discovery_queue: ToolDiscoveryQueue,
    discovery: FakeDiscovery,
    shutdown: CancellationToken,
}

impl Harness {
    fn new(steps: Vec<ChunkStep>) -> Self {
        let events = Arc::new(Events::default());
        let daily_writer = Arc::new(FakeDailyWriter::new(Arc::clone(&events), steps));
        let parts = Arc::new(DailyCommitParts {
            writer: Arc::clone(&daily_writer),
            retries: 0,
            clock: Arc::new(FixedClock {
                instant: Instant::now(),
            }),
            jitter: Arc::new(FixedJitter),
            pause: Arc::new(PauseLog::default()),
            failures: Arc::new(NoSpendLogFailureSink),
        });
        Self {
            entity_queue: spend_queue(),
            entity_writer: FakeEntityWriter::new(Arc::clone(&events)),
            cache: FakeCache::new(Arc::clone(&events)),
            clock: FixedClock {
                instant: Instant::now(),
            },
            jitter: FixedJitter,
            pause: PauseLog::default(),
            failures: NoSpendLogFailureSink,
            daily: daily_queues(),
            commits: TaskTracker::new(),
            parts,
            window_queue: WindowSpendUpdateQueue::new(100, 1000, 100).unwrap(),
            window: FakeWindow::new(Arc::clone(&events)),
            discovery_queue: ToolDiscoveryQueue::new(100).unwrap(),
            discovery: FakeDiscovery::new(Arc::clone(&events)),
            shutdown: CancellationToken::new(),
            events,
        }
    }

    async fn flush(&self) -> Result<(), MemoryFlushError<FakeEntityError, FakeCacheError>> {
        let daily = DailySpendQueues {
            user: &self.daily[0],
            team: &self.daily[1],
            org: &self.daily[2],
            end_user: &self.daily[3],
            agent: &self.daily[4],
        };
        let policy = SpendRetryPolicy {
            retries: 0,
            clock: &self.clock,
            jitter: &self.jitter,
            pause: &self.pause,
            failures: &self.failures,
        };
        let flush = MemorySpendFlush {
            entity_queue: &self.entity_queue,
            entity_writer: &self.entity_writer,
            entity_cache: &self.cache,
            policy,
            daily,
            daily_commits: &self.commits,
            daily_parts: &self.parts,
            window_queue: &self.window_queue,
            window: &self.window,
            discovery_queue: &self.discovery_queue,
            discovery: &self.discovery,
            shutdown: &self.shutdown,
        };
        flush.flush().await
    }
}

fn spend_queue() -> SpendUpdateQueue {
    SpendUpdateQueue::new(100, 1000, 100, Arc::new(NoSpendQueueEventSink)).unwrap()
}

fn daily_queues() -> [Arc<DailySpendUpdateQueue>; 5] {
    [
        daily_queue(),
        daily_queue(),
        daily_queue(),
        daily_queue(),
        daily_queue(),
    ]
}

fn daily_queue() -> Arc<DailySpendUpdateQueue> {
    Arc::new(DailySpendUpdateQueue::new(100, 1000, 100, Arc::new(NoSpendQueueEventSink)).unwrap())
}

fn user_increment() -> SpendUpdateQueueItem {
    SpendUpdateQueueItem {
        entity_type: Some(EntityType::User),
        entity_id: Some("u1".to_owned()),
        response_cost: Some(SpendAmount(Decimal::ONE)),
    }
}

fn discovered_tool() -> ToolDiscoveryItem {
    ToolDiscoveryItem {
        tool_name: Some("search".to_owned()),
        ..ToolDiscoveryItem::default()
    }
}

fn window_increment(spend: Decimal) -> WindowSpendTransaction {
    WindowSpendTransaction {
        entity_type: "user".to_owned(),
        entity_id: "u1".to_owned(),
        window_duration: "1d".to_owned(),
        window_start: "2026-01-01T00:00:00Z".to_owned(),
        spend: SpendAmount(spend),
        started_at: None,
    }
}

fn daily_row() -> SpendRow {
    BTreeMap::from([(
        "date".to_owned(),
        SpendColumnValue::Text("2026-01-01".to_owned()),
    )])
}

fn daily_rows(keys: &[&str]) -> DailySpendUpdate {
    keys.iter()
        .map(|key| ((*key).to_owned(), daily_row()))
        .collect()
}

#[tokio::test]
async fn an_entity_failure_stops_the_pass_before_every_other_stage() {
    let harness = Harness::new(Vec::new());
    harness.entity_writer.fail_with(SpendFailure::Other);
    harness
        .entity_queue
        .add_update(user_increment())
        .await
        .unwrap();
    harness.daily[0]
        .add_update(daily_rows(&["user-1"]))
        .await
        .unwrap();
    harness
        .window_queue
        .add_update(window_increment(Decimal::ONE))
        .await
        .unwrap();
    harness.discovery_queue.add_update(discovered_tool()).await;

    let outcome = harness.flush().await;

    assert!(matches!(outcome, Err(MemoryFlushError::Entity(_))));
    assert_eq!(
        harness.events.entries(),
        ["entity user_list_transactions 1"]
    );
    let daily = harness.daily[0]
        .flush_aggregated_daily_rows()
        .await
        .unwrap();
    assert_eq!(daily.len(), 1);
    let entity = harness
        .entity_queue
        .flush_aggregated_transactions()
        .unwrap();
    assert!(entity.user_list_transactions.is_empty());
}

#[tokio::test]
async fn a_failed_daily_commit_returns_its_rows_and_the_flush_goes_on() {
    let harness = Harness::new(vec![ChunkStep::Fail(SpendFailure::Other)]);
    harness.daily[0]
        .add_update(daily_rows(&["user-1", "user-2"]))
        .await
        .unwrap();
    harness
        .window_queue
        .add_update(window_increment(Decimal::ONE))
        .await
        .unwrap();
    harness.discovery_queue.add_update(discovered_tool()).await;

    let outcome = harness.flush().await;

    outcome.unwrap();
    assert_eq!(
        harness.events.entries(),
        ["daily User 2 Fail(Other)", "window 1", "discovery 1"]
    );
    let requeued = harness.daily[0]
        .flush_aggregated_daily_rows()
        .await
        .unwrap();
    assert_eq!(requeued.keys().collect::<Vec<_>>(), ["user-1", "user-2"]);
}

#[tokio::test]
async fn the_daily_queues_commit_after_the_entity_table_in_the_sources_order() {
    let harness = Harness::new(Vec::new());
    harness
        .entity_queue
        .add_update(user_increment())
        .await
        .unwrap();
    harness.daily[0]
        .add_update(daily_rows(&["user-1"]))
        .await
        .unwrap();
    harness.daily[1]
        .add_update(daily_rows(&["team-1"]))
        .await
        .unwrap();
    harness.daily[2]
        .add_update(daily_rows(&["org-1"]))
        .await
        .unwrap();
    harness.daily[3]
        .add_update(daily_rows(&["end-user-1"]))
        .await
        .unwrap();
    harness.daily[4]
        .add_update(daily_rows(&["agent-1"]))
        .await
        .unwrap();

    harness.flush().await.unwrap();

    assert_eq!(
        harness.events.entries(),
        [
            "entity user_list_transactions 1",
            "daily User 1 Commit",
            "daily Team 1 Commit",
            "daily Org 1 Commit",
            "daily EndUser 1 Commit",
            "daily Agent 1 Commit",
        ]
    );
}

#[tokio::test]
async fn an_unreadable_daily_queue_stops_the_pass_before_the_window() {
    let harness = Harness::new(Vec::new());
    let unreadable = BTreeMap::from([("user-1".to_owned(), SpendRow::new())]);
    harness.daily[0]
        .add_update(unreadable.clone())
        .await
        .unwrap();
    harness.daily[0].add_update(unreadable).await.unwrap();
    harness
        .window_queue
        .add_update(window_increment(Decimal::ONE))
        .await
        .unwrap();
    harness.discovery_queue.add_update(discovered_tool()).await;

    let outcome = harness.flush().await;

    assert!(matches!(
        outcome,
        Err(MemoryFlushError::Daily(DailyFlushError::Queue {
            entity: DailySpendEntity::User,
            ..
        }))
    ));
    assert!(harness.events.entries().is_empty());
}

#[tokio::test]
async fn a_requeue_safe_window_failure_returns_its_increments() {
    let harness = Harness::new(Vec::new());
    harness.window.fail_with(SpendFailure::Other);
    harness
        .window_queue
        .add_update(window_increment(Decimal::ONE))
        .await
        .unwrap();
    harness.discovery_queue.add_update(discovered_tool()).await;

    let outcome = harness.flush().await;

    outcome.unwrap();
    assert_eq!(harness.events.entries(), ["window 1", "discovery 1"]);
    assert_eq!(harness.window_queue.flush().unwrap().len(), 1);
}

#[tokio::test]
async fn an_unsafe_window_failure_drops_its_increments() {
    let harness = Harness::new(Vec::new());
    harness.window.fail_with(SpendFailure::Ambiguous);
    harness
        .window_queue
        .add_update(window_increment(Decimal::ONE))
        .await
        .unwrap();
    harness.discovery_queue.add_update(discovered_tool()).await;

    let outcome = harness.flush().await;

    outcome.unwrap();
    assert_eq!(harness.events.entries(), ["window 1", "discovery 1"]);
    assert!(harness.window_queue.flush().unwrap().is_empty());
}

#[tokio::test]
async fn a_window_aggregation_error_stops_the_pass_before_discovery() {
    let harness = Harness::new(Vec::new());
    harness
        .window_queue
        .add_update(window_increment(Decimal::MAX))
        .await
        .unwrap();
    harness
        .window_queue
        .add_update(window_increment(Decimal::MAX))
        .await
        .unwrap();
    harness.discovery_queue.add_update(discovered_tool()).await;

    let outcome = harness.flush().await;

    assert!(matches!(outcome, Err(MemoryFlushError::WindowQueue(_))));
    assert!(harness.events.entries().is_empty());
    assert!(matches!(
        harness.window_queue.flush(),
        Err(WindowQueueError::SpendOverflow)
    ));
}

#[tokio::test]
async fn a_tool_discovery_failure_leaves_the_drained_tools_lost() {
    let harness = Harness::new(Vec::new());
    harness.discovery.fail();
    harness.discovery_queue.add_update(discovered_tool()).await;

    let outcome = harness.flush().await;

    outcome.unwrap();
    assert_eq!(harness.events.entries(), ["discovery 1"]);
    assert_eq!(harness.discovery.tools(), ["search"]);
    assert!(harness.discovery_queue.flush().is_empty());
}

#[tokio::test]
async fn an_empty_flush_reaches_no_stage() {
    let harness = Harness::new(Vec::new());

    harness.flush().await.unwrap();
    assert!(harness.events.entries().is_empty());
}

#[test]
fn every_entity_kind_reaches_its_own_table() {
    let tables = spend_table_transactions(&populated_transactions());
    let expected: SpendTableTransactions = SpendTable::COMMIT_ORDER
        .into_iter()
        .map(|table| (table, Some(expected_amounts(table))))
        .collect();
    assert_eq!(tables, expected);
}

fn populated_transactions() -> SpendUpdateTransactions {
    let mut transactions = SpendUpdateTransactions::default();
    transactions
        .user_list_transactions
        .extend(increments(SpendTable::UserList));
    transactions
        .end_user_list_transactions
        .extend(increments(SpendTable::EndUserList));
    transactions
        .key_list_transactions
        .extend(increments(SpendTable::KeyList));
    transactions
        .team_list_transactions
        .extend(increments(SpendTable::TeamList));
    transactions
        .team_member_list_transactions
        .extend(increments(SpendTable::TeamMemberList));
    transactions
        .org_list_transactions
        .extend(increments(SpendTable::OrgList));
    transactions
        .org_member_list_transactions
        .extend(increments(SpendTable::OrgMemberList));
    transactions
        .project_list_transactions
        .extend(increments(SpendTable::ProjectList));
    transactions
        .tag_list_transactions
        .extend(increments(SpendTable::TagList));
    transactions
        .model_access_group_spend
        .extend(increments(SpendTable::ModelAccessGroupList));
    transactions
        .agent_list_transactions
        .extend(increments(SpendTable::AgentList));
    transactions
}

// One distinct amount per table, so a bucket wired to the wrong table shows.
fn increments(table: SpendTable) -> BTreeMap<String, SpendAmount> {
    expected_amounts(table)
        .into_iter()
        .map(|(id, amount)| (id, SpendAmount(amount)))
        .collect()
}

fn expected_amounts(table: SpendTable) -> BTreeMap<String, Decimal> {
    let position = SpendTable::COMMIT_ORDER
        .iter()
        .position(|candidate| *candidate == table)
        .unwrap_or_default();
    let amount = Decimal::from(
        u64::try_from(position)
            .unwrap_or_default()
            .saturating_add(1),
    );
    BTreeMap::from([("id".to_owned(), amount)])
}
