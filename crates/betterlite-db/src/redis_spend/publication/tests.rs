#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::too_many_lines,
        reason = "each test reads top to bottom as the scheduler tick it describes: seed the queues, script the pipeline, then assert every cohort"
    )
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use rust_decimal::Decimal;
use serde_json::value::RawValue;

use crate::daily_spend_sql::values::SpendColumnValue;
use crate::mcp_stored_contracts::MAX_STORED_BYTES;
use crate::redis_spend::codec::{decode_daily_batch, decode_entity_batch, decode_window_batch};
use crate::redis_spend::events::RedisBufferEvent;
use crate::redis_spend::keys::{
    REDIS_DAILY_TEAM_BUFFER_KEY, REDIS_DAILY_USER_BUFFER_KEY, REDIS_UPDATE_BUFFER_KEY,
    REDIS_WINDOW_BUFFER_KEY, SpendBufferQueue,
};
use crate::redis_spend::publication::{
    SpendPublishError, SpendUpdateQueues, store_spend_updates_in_redis,
};
use crate::redis_spend::tests::support::{
    FakeLists, FillOnPush, amount, captured_logs, daily_queue, daily_row, detached_publisher,
    merged_daily_row, publisher, publisher_recording, queue_item, transactions, window_queue,
    window_transaction,
};
use crate::spend_queues::daily::DailySpendUpdateQueue;
use crate::spend_queues::entity::{EntityType, SpendUpdateQueue, SpendUpdateTransactions};
use crate::spend_queues::events::NoSpendQueueEventSink;
use crate::window_queue::queue::WindowSpendUpdateQueue;

mod real_redis;

const CAPACITY: i64 = 32;
// Below the capacity so the queue never compacts on its own and the drains stay predictable.
const THRESHOLD: i64 = 16;
const FLUSH: i64 = 32;

struct Fixtures {
    entity: Arc<SpendUpdateQueue>,
    daily_user: Arc<DailySpendUpdateQueue>,
    daily_team: Arc<DailySpendUpdateQueue>,
    daily_org: Arc<DailySpendUpdateQueue>,
    daily_end_user: Arc<DailySpendUpdateQueue>,
    daily_agent: Arc<DailySpendUpdateQueue>,
    window: Arc<WindowSpendUpdateQueue>,
}

impl Fixtures {
    fn new() -> Self {
        Self {
            entity: Arc::new(
                SpendUpdateQueue::new(CAPACITY, THRESHOLD, FLUSH, Arc::new(NoSpendQueueEventSink))
                    .unwrap(),
            ),
            daily_user: Arc::new(daily_queue(CAPACITY, THRESHOLD, FLUSH)),
            daily_team: Arc::new(daily_queue(CAPACITY, THRESHOLD, FLUSH)),
            daily_org: Arc::new(daily_queue(CAPACITY, THRESHOLD, FLUSH)),
            daily_end_user: Arc::new(daily_queue(CAPACITY, THRESHOLD, FLUSH)),
            daily_agent: Arc::new(daily_queue(CAPACITY, THRESHOLD, FLUSH)),
            window: Arc::new(window_queue(CAPACITY, THRESHOLD, FLUSH)),
        }
    }

    fn queues(&self) -> SpendUpdateQueues<'_> {
        SpendUpdateQueues {
            entity: &self.entity,
            daily_user: &self.daily_user,
            daily_team: &self.daily_team,
            daily_org: &self.daily_org,
            daily_end_user: &self.daily_end_user,
            daily_agent: &self.daily_agent,
            window: Some(self.window.as_ref()),
        }
    }

    fn queues_without_window(&self) -> SpendUpdateQueues<'_> {
        SpendUpdateQueues {
            window: None,
            ..self.queues()
        }
    }
}

const fn gauge(queue: SpendBufferQueue, queue_size: i64) -> RedisBufferEvent {
    RedisBufferEvent { queue, queue_size }
}

async fn seed_eleven_kinds(queue: &SpendUpdateQueue) {
    queue
        .add_update(queue_item(EntityType::User, "user-1", "1"))
        .await
        .unwrap();
    queue
        .add_update(queue_item(EntityType::EndUser, "end-user-1", "2"))
        .await
        .unwrap();
    queue
        .add_update(queue_item(EntityType::Key, "key-1", "3"))
        .await
        .unwrap();
    queue
        .add_update(queue_item(EntityType::Team, "team-1", "4"))
        .await
        .unwrap();
    queue
        .add_update(queue_item(EntityType::TeamMember, "team-member-1", "5"))
        .await
        .unwrap();
    queue
        .add_update(queue_item(EntityType::Organization, "org-1", "6"))
        .await
        .unwrap();
    queue
        .add_update(queue_item(
            EntityType::OrganizationMember,
            "org-member-1",
            "7",
        ))
        .await
        .unwrap();
    queue
        .add_update(queue_item(EntityType::Project, "project-1", "8"))
        .await
        .unwrap();
    queue
        .add_update(queue_item(EntityType::Tag, "tag-1", "9"))
        .await
        .unwrap();
    queue
        .add_update(queue_item(EntityType::Agent, "agent-1", "10"))
        .await
        .unwrap();
    queue
        .add_update(queue_item(EntityType::ModelAccessGroup, "group-1", "11"))
        .await
        .unwrap();
}

fn eleven_kinds_transactions() -> SpendUpdateTransactions {
    SpendUpdateTransactions {
        user_list_transactions: BTreeMap::from([("user-1".to_owned(), amount("1"))]),
        end_user_list_transactions: BTreeMap::from([("end-user-1".to_owned(), amount("2"))]),
        key_list_transactions: BTreeMap::from([("key-1".to_owned(), amount("3"))]),
        team_list_transactions: BTreeMap::from([("team-1".to_owned(), amount("4"))]),
        team_member_list_transactions: BTreeMap::from([("team-member-1".to_owned(), amount("5"))]),
        org_list_transactions: BTreeMap::from([("org-1".to_owned(), amount("6"))]),
        org_member_list_transactions: BTreeMap::from([("org-member-1".to_owned(), amount("7"))]),
        project_list_transactions: BTreeMap::from([("project-1".to_owned(), amount("8"))]),
        tag_list_transactions: BTreeMap::from([("tag-1".to_owned(), amount("9"))]),
        agent_list_transactions: BTreeMap::from([("agent-1".to_owned(), amount("10"))]),
        model_access_group_spend: BTreeMap::from([("group-1".to_owned(), amount("11"))]),
    }
}

#[tokio::test]
async fn without_a_redis_client_publication_leaves_memory_untouched() {
    let publisher = detached_publisher();
    let fixtures = Fixtures::new();
    fixtures
        .entity
        .add_update(queue_item(EntityType::User, "user-1", "1.5"))
        .await
        .unwrap();
    fixtures
        .daily_user
        .add_update(daily_row("day-1", "1.5", 1))
        .await
        .unwrap();
    fixtures
        .window
        .add_update(window_transaction("key", "key-1", "1.5"))
        .await
        .unwrap();
    store_spend_updates_in_redis(&publisher, fixtures.queues())
        .await
        .unwrap();
    assert_eq!(
        fixtures.entity.flush_aggregated_transactions().unwrap(),
        transactions(&[("user-1", "1.5")])
    );
    assert_eq!(
        fixtures
            .daily_user
            .flush_aggregated_daily_rows()
            .await
            .unwrap(),
        daily_row("day-1", "1.5", 1)
    );
    assert_eq!(
        fixtures.window.flush().unwrap(),
        vec![window_transaction("key", "key-1", "1.5")]
    );
}

#[tokio::test]
async fn publication_drains_every_queue_into_one_pipeline_in_order() {
    let lists = Arc::new(FakeLists::default());
    let (publisher, events) = publisher_recording(&lists);
    lists.script_push_lengths(vec![11, 3, 5, 7]);
    let fixtures = Fixtures::new();
    fixtures
        .entity
        .add_update(queue_item(EntityType::User, "user-1", "1.5"))
        .await
        .unwrap();
    fixtures
        .daily_user
        .add_update(daily_row("day-1", "1.5", 1))
        .await
        .unwrap();
    fixtures
        .daily_team
        .add_update(daily_row("day-2", "2.5", 2))
        .await
        .unwrap();
    fixtures
        .window
        .add_update(window_transaction("key", "key-1", "3.5"))
        .await
        .unwrap();
    store_spend_updates_in_redis(&publisher, fixtures.queues())
        .await
        .unwrap();
    let pipelines = lists.pushed_pipelines();
    assert_eq!(pipelines.len(), 1);
    let operations = &pipelines[0];
    let keys: Vec<&str> = operations
        .iter()
        .map(|operation| operation.key.as_str())
        .collect();
    assert_eq!(
        keys,
        [
            REDIS_UPDATE_BUFFER_KEY,
            REDIS_DAILY_USER_BUFFER_KEY,
            REDIS_DAILY_TEAM_BUFFER_KEY,
            REDIS_WINDOW_BUFFER_KEY,
        ]
    );
    assert!(
        operations
            .iter()
            .all(|operation| operation.values.len() == 1)
    );
    assert_eq!(
        decode_entity_batch(operations[0].values[0].as_bytes()).unwrap(),
        transactions(&[("user-1", "1.5")])
    );
    assert_eq!(
        decode_daily_batch(operations[1].values[0].as_bytes()).unwrap(),
        daily_row("day-1", "1.5", 1)
    );
    assert_eq!(
        decode_daily_batch(operations[2].values[0].as_bytes()).unwrap(),
        daily_row("day-2", "2.5", 2)
    );
    assert_eq!(
        decode_window_batch(operations[3].values[0].as_bytes()).unwrap(),
        vec![window_transaction("key", "key-1", "3.5")]
    );
    assert_eq!(
        events.events(),
        vec![
            gauge(SpendBufferQueue::Entity, 11),
            gauge(SpendBufferQueue::DailyUser, 3),
            gauge(SpendBufferQueue::DailyTeam, 5),
            gauge(SpendBufferQueue::Window, 7),
        ]
    );
    assert_eq!(
        fixtures.entity.flush_aggregated_transactions().unwrap(),
        SpendUpdateTransactions::default(),
        "a successful push must leave nothing in memory"
    );
}

#[tokio::test]
async fn a_publication_without_a_window_queue_pushes_only_the_entity_cohort() {
    let lists = Arc::new(FakeLists::default());
    let (publisher, events) = publisher_recording(&lists);
    lists.script_push_lengths(vec![11]);
    let fixtures = Fixtures::new();
    fixtures
        .window
        .add_update(window_transaction("key", "key-1", "3.5"))
        .await
        .unwrap();
    store_spend_updates_in_redis(&publisher, fixtures.queues_without_window())
        .await
        .unwrap();
    let pipelines = lists.pushed_pipelines();
    let keys: Vec<&str> = pipelines[0]
        .iter()
        .map(|operation| operation.key.as_str())
        .collect();
    assert_eq!(keys, [REDIS_UPDATE_BUFFER_KEY]);
    assert_eq!(events.events(), vec![gauge(SpendBufferQueue::Entity, 11)]);
    assert_eq!(
        fixtures.window.flush().unwrap(),
        vec![window_transaction("key", "key-1", "3.5")],
        "without a window queue the publisher must neither drain nor publish the window cohort"
    );
}

#[tokio::test]
async fn the_entity_cohort_is_pushed_even_with_every_bucket_empty() {
    let lists = Arc::new(FakeLists::default());
    let (publisher, events) = publisher_recording(&lists);
    lists.script_push_lengths(vec![11]);
    let fixtures = Fixtures::new();
    store_spend_updates_in_redis(&publisher, fixtures.queues())
        .await
        .unwrap();
    let pipelines = lists.pushed_pipelines();
    assert_eq!(pipelines.len(), 1);
    let operations = &pipelines[0];
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].key, REDIS_UPDATE_BUFFER_KEY);
    let fields: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(&operations[0].values[0]).unwrap();
    assert_eq!(fields.len(), 11);
    assert!(fields.contains_key("model_access_group_list_transactions"));
    assert_eq!(
        decode_entity_batch(operations[0].values[0].as_bytes()).unwrap(),
        SpendUpdateTransactions::default()
    );
    assert_eq!(events.events(), vec![gauge(SpendBufferQueue::Entity, 11)]);
}

#[tokio::test]
async fn a_failed_pipeline_restores_every_drained_cohort() {
    let (logs, _guard) = captured_logs();
    let lists = Arc::new(FakeLists::default());
    let publisher = publisher(&lists);
    lists.fail_push();
    let fixtures = Fixtures::new();
    fixtures
        .entity
        .add_update(queue_item(EntityType::User, "user-1", "1.5"))
        .await
        .unwrap();
    fixtures
        .entity
        .add_update(queue_item(EntityType::Key, "key-1", "0.5"))
        .await
        .unwrap();
    fixtures
        .daily_user
        .add_update(daily_row("day-1", "1.5", 1))
        .await
        .unwrap();
    fixtures
        .daily_team
        .add_update(daily_row("day-2", "2.5", 2))
        .await
        .unwrap();
    fixtures
        .window
        .add_update(window_transaction("key", "key-1", "3.5"))
        .await
        .unwrap();
    store_spend_updates_in_redis(&publisher, fixtures.queues())
        .await
        .unwrap();
    let text = logs.text();
    assert!(text.contains("failed to push aggregated spend updates to Redis"));
    assert!(text.contains("Restoring 4 transaction sets"), "{text}");
    let mut expected = transactions(&[("user-1", "1.5")]);
    expected
        .key_list_transactions
        .insert("key-1".to_owned(), amount("0.5"));
    assert_eq!(
        fixtures.entity.flush_aggregated_transactions().unwrap(),
        expected
    );
    assert_eq!(
        fixtures
            .daily_user
            .flush_aggregated_daily_rows()
            .await
            .unwrap(),
        daily_row("day-1", "1.5", 1)
    );
    assert_eq!(
        fixtures
            .daily_team
            .flush_aggregated_daily_rows()
            .await
            .unwrap(),
        daily_row("day-2", "2.5", 2)
    );
    assert_eq!(
        fixtures.window.flush().unwrap(),
        vec![window_transaction("key", "key-1", "3.5")]
    );
}

#[tokio::test]
async fn a_failed_pipeline_returns_all_eleven_entity_kinds_to_the_queue() {
    let lists = Arc::new(FakeLists::default());
    let publisher = publisher(&lists);
    lists.fail_push();
    let fixtures = Fixtures::new();
    seed_eleven_kinds(&fixtures.entity).await;
    store_spend_updates_in_redis(&publisher, fixtures.queues())
        .await
        .unwrap();
    assert_eq!(
        fixtures.entity.flush_aggregated_transactions().unwrap(),
        eleven_kinds_transactions()
    );
}

#[tokio::test]
async fn an_encoding_failure_propagates_before_the_pipeline_and_loses_the_drain() {
    let lists = Arc::new(FakeLists::default());
    let publisher = publisher(&lists);
    let fixtures = Fixtures::new();
    fixtures
        .entity
        .add_update(queue_item(EntityType::User, "user-1", "1.5"))
        .await
        .unwrap();
    let oversized = "x".repeat(MAX_STORED_BYTES + 1);
    fixtures
        .daily_user
        .add_update(daily_row(&oversized, "1.5", 1))
        .await
        .unwrap();
    let error = store_spend_updates_in_redis(&publisher, fixtures.queues())
        .await
        .unwrap_err();
    assert!(matches!(error, SpendPublishError::Encode(_)), "{error:?}");
    assert!(
        lists.pushed_pipelines().is_empty(),
        "encoding runs before the pipeline is attempted"
    );
    assert_eq!(
        fixtures.entity.flush_aggregated_transactions().unwrap(),
        SpendUpdateTransactions::default(),
        "an encoding failure is outside the pipeline's recovery branch"
    );
}

#[tokio::test]
async fn gauges_follow_the_returned_lengths_only() {
    let lists = Arc::new(FakeLists::default());
    let (publisher, events) = publisher_recording(&lists);
    let fixtures = Fixtures::new();
    fixtures
        .entity
        .add_update(queue_item(EntityType::User, "user-1", "1.5"))
        .await
        .unwrap();
    fixtures
        .daily_user
        .add_update(daily_row("day-1", "1.5", 1))
        .await
        .unwrap();
    fixtures
        .window
        .add_update(window_transaction("key", "key-1", "3.5"))
        .await
        .unwrap();
    lists.script_push_lengths(vec![11, 30, 50, 70]);
    store_spend_updates_in_redis(&publisher, fixtures.queues())
        .await
        .unwrap();
    assert_eq!(
        events.events(),
        vec![
            gauge(SpendBufferQueue::Entity, 11),
            gauge(SpendBufferQueue::DailyUser, 30),
            gauge(SpendBufferQueue::Window, 50),
        ],
        "a length without a pushed queue is ignored"
    );
    lists.script_push_lengths(Vec::new());
    store_spend_updates_in_redis(&publisher, fixtures.queues())
        .await
        .unwrap();
    assert_eq!(
        events.events().len(),
        3,
        "a pushed queue without a returned length gets no gauge"
    );
    let pipelines = lists.pushed_pipelines();
    assert_eq!(pipelines.len(), 2);
    assert_eq!(pipelines[1][0].key, REDIS_UPDATE_BUFFER_KEY);
}

#[tokio::test]
async fn a_full_daily_queue_stages_the_restored_cohort_for_the_next_tick() {
    let lists = Arc::new(FakeLists::default());
    let publisher = publisher(&lists);
    let mut fixtures = Fixtures::new();
    fixtures.daily_user = Arc::new(daily_queue(1, 0, 1));
    fixtures
        .daily_user
        .add_update(daily_row("day-1", "1.0", 1))
        .await
        .unwrap();
    lists.fill_on_push(FillOnPush::Daily(
        Arc::clone(&fixtures.daily_user),
        daily_row("day-1", "2.0", 2),
    ));
    lists.fail_push();
    store_spend_updates_in_redis(&publisher, fixtures.queues())
        .await
        .unwrap();
    let held = fixtures
        .daily_user
        .flush_aggregated_daily_rows()
        .await
        .unwrap();
    assert_eq!(held, daily_row("day-1", "2.0", 2));
    fixtures
        .daily_user
        .add_update(daily_row("day-1", "4.0", 4))
        .await
        .unwrap();
    lists.allow_push();
    lists.script_push_lengths(vec![11, 3]);
    store_spend_updates_in_redis(&publisher, fixtures.queues())
        .await
        .unwrap();
    let pipelines = lists.pushed_pipelines();
    assert_eq!(pipelines.len(), 2);
    let daily = pipelines[1]
        .iter()
        .find(|operation| operation.key == REDIS_DAILY_USER_BUFFER_KEY)
        .unwrap();
    assert_eq!(
        decode_daily_batch(daily.values[0].as_bytes()).unwrap(),
        merged_daily_row("day-1", "5.0", 5),
        "the staged cohort joins the next publication's aggregate"
    );
}

#[tokio::test]
async fn a_full_window_queue_stages_restored_increments_for_the_next_tick() {
    let lists = Arc::new(FakeLists::default());
    let publisher = publisher(&lists);
    let mut fixtures = Fixtures::new();
    fixtures.window = Arc::new(window_queue(1, 0, 1));
    fixtures
        .window
        .add_update(window_transaction("key", "key-1", "1.5"))
        .await
        .unwrap();
    lists.fill_on_push(FillOnPush::Window(
        Arc::clone(&fixtures.window),
        window_transaction("key", "key-2", "2.5"),
    ));
    lists.fail_push();
    store_spend_updates_in_redis(&publisher, fixtures.queues())
        .await
        .unwrap();
    assert_eq!(
        fixtures.window.flush().unwrap(),
        vec![window_transaction("key", "key-2", "2.5")]
    );
    fixtures
        .window
        .add_update(window_transaction("key", "key-3", "3.5"))
        .await
        .unwrap();
    lists.allow_push();
    lists.script_push_lengths(vec![11, 3]);
    store_spend_updates_in_redis(&publisher, fixtures.queues())
        .await
        .unwrap();
    let pipelines = lists.pushed_pipelines();
    assert_eq!(pipelines.len(), 2);
    let window = pipelines[1]
        .iter()
        .find(|operation| operation.key == REDIS_WINDOW_BUFFER_KEY)
        .unwrap();
    assert_eq!(
        decode_window_batch(window.values[0].as_bytes()).unwrap(),
        vec![
            window_transaction("key", "key-1", "1.5"),
            window_transaction("key", "key-3", "3.5"),
        ],
        "the staged increment joins the next publication's aggregate"
    );
}

#[tokio::test]
async fn the_published_daily_row_keeps_its_decimal_money() {
    let lists = Arc::new(FakeLists::default());
    let publisher = publisher(&lists);
    lists.script_push_lengths(vec![11, 3]);
    let fixtures = Fixtures::new();
    fixtures
        .daily_user
        .add_update(daily_row("day-1", "0.30000000000000004", 1))
        .await
        .unwrap();
    store_spend_updates_in_redis(&publisher, fixtures.queues())
        .await
        .unwrap();
    let pipelines = lists.pushed_pipelines();
    let daily = pipelines[0]
        .iter()
        .find(|operation| operation.key == REDIS_DAILY_USER_BUFFER_KEY)
        .unwrap();
    let decoded = decode_daily_batch(daily.values[0].as_bytes()).unwrap();
    assert_eq!(
        decoded["day-1"]["spend"],
        SpendColumnValue::Decimal("0.30000000000000004".parse::<Decimal>().unwrap())
    );
}
