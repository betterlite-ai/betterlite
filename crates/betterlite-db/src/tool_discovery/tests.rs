#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;
use tokio::time::timeout;

use super::{ToolDiscoveryError, ToolDiscoveryItem, ToolDiscoveryQueue};

fn named(name: &str) -> ToolDiscoveryItem {
    ToolDiscoveryItem {
        tool_name: Some(name.to_owned()),
        origin: None,
        created_by: None,
        key_hash: None,
        team_id: None,
        key_alias: None,
        user_agent: None,
    }
}

fn first_occurrence(name: &str) -> ToolDiscoveryItem {
    let mut first = named(name);
    first.origin = Some(String::from("openai"));
    first.created_by = Some(String::from("user-1"));
    first.key_hash = Some(String::from("hash-1"));
    first.team_id = Some(String::from("team-1"));
    first.key_alias = Some(String::from("alias-1"));
    first.user_agent = Some(String::from("curl/8.4.0"));
    first
}

fn later_occurrence(name: &str) -> ToolDiscoveryItem {
    let mut later = named(name);
    later.origin = Some(String::from("anthropic"));
    later
}

#[test]
fn an_item_without_a_name_is_not_a_tool() {
    let item: ToolDiscoveryItem =
        serde_json::from_str(r#"{"tool_name": null, "origin": "openai"}"#).expect("an item");
    assert_eq!(
        item,
        ToolDiscoveryItem {
            tool_name: None,
            origin: Some(String::from("openai")),
            created_by: None,
            key_hash: None,
            team_id: None,
            key_alias: None,
            user_agent: None,
        }
    );
    let absent: ToolDiscoveryItem =
        serde_json::from_str(r#"{"origin": "openai"}"#).expect("an item");
    assert_eq!(absent, item);
}

#[tokio::test]
async fn a_nameless_item_is_skipped() {
    let queue = ToolDiscoveryQueue::new(1).expect("a queue");
    queue.add_update(ToolDiscoveryItem::default()).await;
    queue.add_update(named("")).await;
    assert!(queue.flush().is_empty());
}

#[tokio::test]
async fn the_first_item_for_each_tool_name_keeps_every_field() {
    let queue = ToolDiscoveryQueue::new(8).expect("a queue");
    queue.add_update(first_occurrence("search")).await;
    queue.add_update(later_occurrence("search")).await;
    queue.add_update(first_occurrence("fetch")).await;
    assert_eq!(
        queue.flush(),
        [first_occurrence("search"), first_occurrence("fetch")]
    );
}

#[tokio::test]
async fn a_blank_name_is_kept_untrimmed_once_per_cycle() {
    let queue = ToolDiscoveryQueue::new(2).expect("a queue");
    queue.add_update(named(" ")).await;
    queue.add_update(named(" ")).await;
    assert_eq!(queue.flush(), [named(" ")]);
}

#[tokio::test]
async fn a_name_counts_again_in_the_next_cycle() {
    let queue = ToolDiscoveryQueue::new(4).expect("a queue");
    queue.add_update(first_occurrence("search")).await;
    assert_eq!(queue.flush(), [first_occurrence("search")]);
    queue.add_update(later_occurrence("search")).await;
    assert_eq!(queue.flush(), [later_occurrence("search")]);
}

#[tokio::test]
async fn a_full_buffer_waits_for_a_flush_without_dropping_the_tool() {
    let queue = ToolDiscoveryQueue::new(1).expect("a queue");
    queue.add_update(named("search")).await;
    timeout(
        Duration::from_secs(1),
        queue.add_update(later_occurrence("search")),
    )
    .await
    .expect("a queued name is skipped without waiting");
    timeout(Duration::from_millis(20), queue.add_update(named("fetch")))
        .await
        .unwrap_err();
    assert_eq!(queue.flush(), [named("search")]);
    timeout(Duration::from_secs(1), queue.add_update(named("fetch")))
        .await
        .expect("the flush freed the buffer");
    assert_eq!(queue.flush(), [named("fetch")]);
}

#[tokio::test]
async fn a_flush_wakes_a_waiter_for_room() {
    let queue = Arc::new(ToolDiscoveryQueue::new(1).expect("a queue"));
    queue.add_update(named("search")).await;
    let waiting = Arc::clone(&queue);
    let mut producers = JoinSet::new();
    producers.spawn(async move { waiting.add_update(named("fetch")).await });
    tokio::task::yield_now().await;
    assert_eq!(queue.flush(), [named("search")]);
    timeout(Duration::from_secs(1), producers.join_next())
        .await
        .expect("the flush wakes the producer")
        .expect("the producer task finished")
        .expect("the producer queued the tool");
    assert_eq!(queue.flush(), [named("fetch")]);
}

#[test]
fn an_empty_flush_returns_nothing() {
    let queue = ToolDiscoveryQueue::new(4).expect("a queue");
    assert!(queue.flush().is_empty());
}

#[test]
fn a_capacity_of_at_least_one_is_required() {
    assert!(matches!(
        ToolDiscoveryQueue::new(0),
        Err(ToolDiscoveryError::ZeroCapacity)
    ));
    assert!(matches!(
        ToolDiscoveryQueue::new(-5),
        Err(ToolDiscoveryError::ZeroCapacity)
    ));
    ToolDiscoveryQueue::new(1).expect("a queue with one slot");
}
