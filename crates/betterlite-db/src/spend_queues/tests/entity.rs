use std::sync::Arc;
use std::time::Duration;

use rust_decimal::Decimal;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::support::{RecordingSink, amount, capture_logs, item};
use crate::spend_queues::SpendQueueError;
use crate::spend_queues::entity::{
    EntityType, SpendAmount, SpendUpdateQueue, SpendUpdateQueueItem, SpendUpdateTransactions,
    aggregate_db_spend_update_transactions, aggregate_spend_update_queue_items,
};
use crate::spend_queues::events::{NoSpendQueueEventSink, SpendQueueService};

fn huge(kind: EntityType, entity_id: &str) -> SpendUpdateQueueItem {
    SpendUpdateQueueItem {
        entity_type: Some(kind),
        entity_id: Some(entity_id.to_owned()),
        response_cost: Some(SpendAmount(Decimal::MAX)),
    }
}

fn one(kind: EntityType, entity_id: &str) -> SpendUpdateQueueItem {
    SpendUpdateQueueItem {
        entity_type: Some(kind),
        entity_id: Some(entity_id.to_owned()),
        response_cost: Some(SpendAmount(Decimal::ONE)),
    }
}

#[test]
fn compaction_groups_by_kind_and_id_in_first_seen_order_and_adds_costs() {
    let aggregated = aggregate_spend_update_queue_items(&[
        item(Some(EntityType::User), Some("1"), Some("10.5")),
        item(Some(EntityType::Key), Some("k"), Some("1.0")),
        item(Some(EntityType::User), Some("1"), Some("2.5")),
        item(Some(EntityType::User), Some("2"), None),
    ])
    .unwrap();
    assert_eq!(
        aggregated,
        vec![
            item(Some(EntityType::User), Some("1"), Some("13.0")),
            item(Some(EntityType::Key), Some("k"), Some("1.0")),
            item(Some(EntityType::User), Some("2"), None),
        ]
    );
}

#[test]
fn a_first_record_without_a_cost_gains_zero_and_a_singleton_keeps_its_absence() {
    let aggregated = aggregate_spend_update_queue_items(&[
        item(Some(EntityType::User), Some("1"), None),
        item(Some(EntityType::User), Some("1"), Some("5")),
        item(Some(EntityType::User), Some("2"), Some("0")),
        item(Some(EntityType::User), Some("2"), None),
    ])
    .unwrap();
    assert_eq!(
        aggregated,
        vec![
            item(Some(EntityType::User), Some("1"), Some("5")),
            item(Some(EntityType::User), Some("2"), Some("0")),
        ]
    );
}

#[test]
fn costs_are_summed_as_exact_decimals() {
    let aggregated = aggregate_spend_update_queue_items(&[
        item(Some(EntityType::User), Some("1"), Some("0.1")),
        item(Some(EntityType::User), Some("1"), Some("0.2")),
    ])
    .unwrap();
    assert_eq!(aggregated[0].response_cost, Some(amount("0.3")));
}

#[test]
fn the_same_id_under_different_kinds_stays_apart() {
    let aggregated = aggregate_spend_update_queue_items(&[
        item(Some(EntityType::User), Some("1"), Some("1.0")),
        item(Some(EntityType::Key), Some("1"), Some("2.0")),
    ])
    .unwrap();
    assert_eq!(aggregated.len(), 2);
}

#[test]
fn compaction_leaves_its_inputs_untouched() {
    let updates = vec![
        item(Some(EntityType::User), Some("1"), Some("1.5")),
        item(Some(EntityType::User), Some("1"), Some("2.5")),
        item(Some(EntityType::Team), None, Some("3.5")),
    ];
    let original = updates.clone();
    let aggregated = aggregate_spend_update_queue_items(&updates).unwrap();
    assert_eq!(updates, original);
    assert_eq!(aggregated.len(), 2);
    assert_eq!(
        aggregated[0].response_cost,
        Some(amount("4.0")),
        "the aggregated copy carries the sum"
    );
}

#[test]
fn a_cost_that_overflows_the_decimal_range_is_reported() {
    let updates = vec![huge(EntityType::User, "1"), one(EntityType::User, "1")];
    assert!(matches!(
        aggregate_spend_update_queue_items(&updates),
        Err(SpendQueueError::CostOverflow)
    ));
}

fn every_kind_updates() -> Vec<SpendUpdateQueueItem> {
    vec![
        item(Some(EntityType::Key), Some("k"), Some("1")),
        item(Some(EntityType::User), Some("u"), Some("2")),
        item(Some(EntityType::EndUser), Some("e"), Some("3")),
        item(Some(EntityType::Team), Some("t"), Some("4")),
        item(Some(EntityType::TeamMember), Some("tm"), Some("5")),
        item(Some(EntityType::Organization), Some("o"), Some("6")),
        item(Some(EntityType::OrganizationMember), Some("om"), Some("7")),
        item(Some(EntityType::Project), Some("p"), Some("8")),
        item(Some(EntityType::Tag), Some("g"), Some("9")),
        item(Some(EntityType::Agent), Some("a"), Some("10")),
        item(Some(EntityType::ModelAccessGroup), Some("mag"), Some("11")),
        item(Some(EntityType::User), Some("u"), Some("12")),
        item(Some(EntityType::Team), None, Some("1")),
        item(Some(EntityType::Team), Some(""), Some("2")),
        item(Some(EntityType::Key), Some("k"), None),
        item(None, Some("missing"), Some("1")),
        item(Some(EntityType::Proxy), Some("px"), Some("1")),
        item(
            Some(EntityType::Unsupported("window".to_owned())),
            Some("w"),
            Some("1"),
        ),
    ]
}

fn expected_transactions() -> SpendUpdateTransactions {
    let mut expected = SpendUpdateTransactions::default();
    expected
        .key_list_transactions
        .insert("k".to_owned(), amount("1"));
    expected
        .user_list_transactions
        .insert("u".to_owned(), amount("14"));
    expected
        .end_user_list_transactions
        .insert("e".to_owned(), amount("3"));
    expected
        .team_list_transactions
        .insert("t".to_owned(), amount("4"));
    expected
        .team_list_transactions
        .insert(String::new(), amount("3"));
    expected
        .team_member_list_transactions
        .insert("tm".to_owned(), amount("5"));
    expected
        .org_list_transactions
        .insert("o".to_owned(), amount("6"));
    expected
        .org_member_list_transactions
        .insert("om".to_owned(), amount("7"));
    expected
        .project_list_transactions
        .insert("p".to_owned(), amount("8"));
    expected
        .tag_list_transactions
        .insert("g".to_owned(), amount("9"));
    expected
        .agent_list_transactions
        .insert("a".to_owned(), amount("10"));
    expected
        .model_access_group_list_transactions
        .insert("mag".to_owned(), amount("11"));
    expected
}

#[test]
fn database_aggregation_builds_every_bucket_and_skips_kinds_without_one() {
    let transactions = aggregate_db_spend_update_transactions(&every_kind_updates()).unwrap();
    assert_eq!(transactions, expected_transactions());
}

#[test]
fn database_aggregation_reports_a_cost_that_overflows_the_decimal_range() {
    let updates = vec![huge(EntityType::Team, "t"), one(EntityType::Team, "t")];
    assert!(matches!(
        aggregate_db_spend_update_transactions(&updates),
        Err(SpendQueueError::CostOverflow)
    ));
}

async fn add_user_cost(queue: &SpendUpdateQueue, cost: &str) {
    queue
        .add_update(item(Some(EntityType::User), Some("1"), Some(cost)))
        .await
        .unwrap();
}

#[tokio::test]
async fn add_update_compacts_at_the_threshold_and_reports_the_compacted_size() {
    let sink = Arc::new(RecordingSink::default());
    let queue = SpendUpdateQueue::new(8, 2, 10, Arc::clone(&sink)).unwrap();
    add_user_cost(&queue, "1.5").await;
    add_user_cost(&queue, "2.5").await;
    add_user_cost(&queue, "4.0").await;
    let transactions = queue
        .flush_and_get_aggregated_db_spend_update_transactions()
        .unwrap();
    assert_eq!(
        transactions.user_list_transactions.get("1"),
        Some(&amount("8.0"))
    );
    queue.shutdown().await;

    let events = sink.events();
    assert_eq!(events.len(), 3);
    assert!(
        events
            .iter()
            .all(|event| event.service == SpendQueueService::InMemorySpendUpdateQueue)
    );
    let mut sizes = events
        .iter()
        .map(|event| event.queue_size)
        .collect::<Vec<_>>();
    sizes.sort_unstable();
    assert_eq!(sizes, vec![1, 2, 2]);
}

#[tokio::test]
async fn a_flush_logs_its_item_count_and_keeps_the_update_body_out_of_logs() {
    let logs = capture_logs();
    let queue = SpendUpdateQueue::new(8, 100, 10, Arc::new(NoSpendQueueEventSink)).unwrap();
    queue
        .add_update(item(Some(EntityType::Key), Some("sk-secret"), Some("1.5")))
        .await
        .unwrap();
    queue
        .flush_and_get_aggregated_db_spend_update_transactions()
        .unwrap();
    let text = logs.text();
    assert!(text.contains("Spend tracking - flushed spend update items from in-memory queue"));
    assert!(text.contains("items=1"));
    assert!(!text.contains("sk-secret"));
    queue.shutdown().await;
}

#[tokio::test]
async fn an_empty_flush_builds_every_bucket_without_a_flush_log() {
    let logs = capture_logs();
    let queue = SpendUpdateQueue::new(8, 100, 10, Arc::new(NoSpendQueueEventSink)).unwrap();
    let transactions = queue
        .flush_and_get_aggregated_db_spend_update_transactions()
        .unwrap();
    assert_eq!(transactions, SpendUpdateTransactions::default());
    assert!(!logs.contains("flushed spend update items"));
    queue.shutdown().await;
}

#[tokio::test]
async fn a_full_queue_blocks_a_producer_until_a_flush_drains_it() {
    let queue =
        Arc::new(SpendUpdateQueue::new(1, 100, 10, Arc::new(NoSpendQueueEventSink)).unwrap());
    queue
        .add_update(item(Some(EntityType::User), Some("1"), Some("1.5")))
        .await
        .unwrap();
    let mut tasks = JoinSet::new();
    let producing = Arc::clone(&queue);
    tasks.spawn(async move {
        producing
            .add_update(item(Some(EntityType::User), Some("2"), Some("2.5")))
            .await
            .unwrap();
    });
    assert!(
        timeout(Duration::from_millis(20), tasks.join_next())
            .await
            .is_err()
    );
    let first = queue
        .flush_and_get_aggregated_db_spend_update_transactions()
        .unwrap();
    assert_eq!(first.user_list_transactions.len(), 1);
    timeout(Duration::from_secs(1), tasks.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let second = queue
        .flush_and_get_aggregated_db_spend_update_transactions()
        .unwrap();
    assert_eq!(second.user_list_transactions.get("2"), Some(&amount("2.5")));
    queue.shutdown().await;
}

#[test]
fn queue_items_decode_every_entity_kind_and_the_unknown_boundary() {
    let cases = [
        ("key", EntityType::Key),
        ("user", EntityType::User),
        ("end_user", EntityType::EndUser),
        ("team", EntityType::Team),
        ("team_member", EntityType::TeamMember),
        ("organization", EntityType::Organization),
        ("organization_member", EntityType::OrganizationMember),
        ("project", EntityType::Project),
        ("tag", EntityType::Tag),
        ("agent", EntityType::Agent),
        ("model_access_group", EntityType::ModelAccessGroup),
        ("proxy", EntityType::Proxy),
        ("window", EntityType::Unsupported("window".to_owned())),
    ];
    for (name, expected) in cases {
        let json = format!(r#"{{"entity_type":"{name}"}}"#);
        let item: SpendUpdateQueueItem = serde_json::from_str(&json).unwrap();
        assert_eq!(item.entity_type, Some(expected), "{name}");
    }
}

#[test]
fn spend_amounts_round_trip_through_json_numbers() {
    let item: SpendUpdateQueueItem =
        serde_json::from_str(r#"{"entity_type":"user","entity_id":"u","response_cost":1.25}"#)
            .unwrap();
    assert_eq!(item.response_cost, Some(amount("1.25")));
    assert_eq!(serde_json::to_string(&amount("1.25")).unwrap(), "1.25");
}

#[test]
fn a_falsey_float_cost_is_stored_as_an_integer_zero() {
    let transactions = aggregate_db_spend_update_transactions(&[item(
        Some(EntityType::User),
        Some("u"),
        Some("0.0"),
    )])
    .unwrap();
    let json = serde_json::to_string(&transactions).unwrap();
    assert!(
        json.contains(r#""user_list_transactions":{"u":0}"#),
        "{json}"
    );
}

#[test]
fn a_capacity_of_at_least_one_is_required() {
    let sink = Arc::new(NoSpendQueueEventSink);
    assert!(matches!(
        SpendUpdateQueue::new(0, 0, 1, sink),
        Err(SpendQueueError::ZeroCapacity)
    ));
}
