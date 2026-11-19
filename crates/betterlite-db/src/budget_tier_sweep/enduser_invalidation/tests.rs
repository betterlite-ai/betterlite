#![cfg(test)]

use rust_decimal::Decimal;

use crate::budget_reset_policy::cache_invalidation::InvalidatedCache;
use crate::budget_tier_sweep::enduser_invalidation::{
    EndUserWalkRequest, invalidate_enduser_caches,
};
use crate::budget_tier_sweep::enduser_page_truncation::EndUserWalk;
use crate::budget_tier_sweep::tests::support::{
    CountingReconnect, Harness, RecordingCache, TestSchedule, close_store, finish, insert_tier,
    moment, past, schema, serial_pool, spend_of, walk_five_customers,
};
use crate::postgres_store::session::PostgresStore;

#[tokio::test]
async fn a_complete_walk_counts_every_customer_and_reaches_the_end() {
    let (fixture, store, client) = schema().await;
    let walked = walk_five_customers(&store, &client).await;
    assert_eq!(
        walked.walk,
        EndUserWalk {
            cursor: None,
            invalidated: 5,
            truncated: false,
        }
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn walk_drops_each_full_page_before_reading_the_next() {
    let (fixture, store, client) = schema().await;
    let walked = walk_five_customers(&store, &client).await;
    let calls = walked.cache.calls();
    let sinks: Vec<InvalidatedCache> = calls.iter().map(|call| call.sink).collect();
    assert_eq!(
        sinks,
        vec![
            InvalidatedCache::SpendCounter,
            InvalidatedCache::UserApiKeyCache,
            InvalidatedCache::SpendCounter,
            InvalidatedCache::UserApiKeyCache,
            InvalidatedCache::SpendCounter,
            InvalidatedCache::UserApiKeyCache,
        ]
    );
    let counters: Vec<Vec<String>> = calls
        .iter()
        .filter(|call| call.sink == InvalidatedCache::SpendCounter)
        .map(|call| call.keys.clone())
        .collect();
    assert_eq!(
        counters,
        vec![
            vec!["spend:end_user:u1", "spend:end_user:u2"],
            vec!["spend:end_user:u3", "spend:end_user:u4"],
            vec!["spend:end_user:u5"],
        ]
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn an_empty_selection_reads_no_customer_and_drops_nothing() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_EndUserTable\" (user_id, budget_id, spend) VALUES ('unlinked', NULL, 10)",
        )
        .await
        .unwrap();
    let cache = RecordingCache::default();
    let budget_ids: Vec<String> = Vec::new();
    let request = EndUserWalkRequest {
        budget_ids: &budget_ids,
        default_enduser_budget_id: Some("tier-a"),
        batch_size: 500,
    };
    let mut reconnect = CountingReconnect::new(false);
    let walk = invalidate_enduser_caches(&store, &cache, request, Some(&mut reconnect)).await;
    assert_eq!(
        walk,
        EndUserWalk {
            cursor: None,
            invalidated: 0,
            truncated: false,
        }
    );
    assert!(cache.calls().is_empty());
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn a_page_read_retries_after_reconnecting() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_EndUserTable\" (user_id, budget_id, spend) VALUES ('u1', 'tier-a', 10)",
        )
        .await
        .unwrap();
    let serial = PostgresStore {
        pool: Some(serial_pool(&fixture.options.url)),
    };
    let held = serial.session().await.unwrap();
    let cache = RecordingCache::default();
    let budget_ids = vec![String::from("tier-a")];
    let request = EndUserWalkRequest {
        budget_ids: &budget_ids,
        default_enduser_budget_id: None,
        batch_size: 2,
    };
    let mut reconnect = CountingReconnect::releasing(held);
    let walk = invalidate_enduser_caches(&serial, &cache, request, Some(&mut reconnect)).await;
    assert_eq!(walk.invalidated, 1);
    assert_eq!(reconnect.attempts(), 1);
    assert_eq!(
        reconnect.reasons(),
        vec![String::from("reset_budget_read_endusers_failure")]
    );
    close_store(serial);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn the_default_tier_walk_takes_unlinked_customers() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_EndUserTable\" (user_id, budget_id, spend) VALUES ('linked', 'tier-a', 10), ('unlinked', NULL, 10), ('other', 'tier-b', 10)",
        )
        .await
        .unwrap();
    let cache = RecordingCache::default();
    let budget_ids = vec![String::from("tier-a")];
    let request = EndUserWalkRequest {
        budget_ids: &budget_ids,
        default_enduser_budget_id: Some("tier-a"),
        batch_size: 500,
    };
    let mut reconnect = CountingReconnect::new(false);
    let walk = invalidate_enduser_caches(&store, &cache, request, Some(&mut reconnect)).await;
    assert_eq!(walk.invalidated, 2);
    let keys: Vec<String> = cache
        .calls()
        .into_iter()
        .filter(|call| call.sink == InvalidatedCache::SpendCounter)
        .flat_map(|call| call.keys)
        .collect();
    assert_eq!(
        keys,
        vec![
            "spend:end_user:linked".to_owned(),
            "spend:end_user:unlinked".to_owned(),
        ]
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn a_plain_tier_walk_leaves_unlinked_customers_alone() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_EndUserTable\" (user_id, budget_id, spend) VALUES ('linked', 'tier-a', 10), ('unlinked', NULL, 10), ('other', 'tier-b', 10)",
        )
        .await
        .unwrap();
    let cache = RecordingCache::default();
    let budget_ids = vec![String::from("tier-a")];
    let request = EndUserWalkRequest {
        budget_ids: &budget_ids,
        default_enduser_budget_id: None,
        batch_size: 500,
    };
    let mut reconnect = CountingReconnect::new(false);
    let walk = invalidate_enduser_caches(&store, &cache, request, Some(&mut reconnect)).await;
    assert_eq!(walk.invalidated, 1);
    let keys: Vec<String> = cache
        .calls()
        .into_iter()
        .filter(|call| call.sink == InvalidatedCache::SpendCounter)
        .flat_map(|call| call.keys)
        .collect();
    assert_eq!(keys, vec!["spend:end_user:linked".to_owned()]);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn the_default_tier_service_walk_reaches_unlinked_customers() {
    let (fixture, store, client) = schema().await;
    insert_tier(&client, "tier-a", Some("1d"), Some(past())).await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_EndUserTable\" (user_id, budget_id, spend) VALUES ('unlinked', NULL, 10)",
        )
        .await
        .unwrap();
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.default_enduser_budget_id = Some(String::from("tier-a"));
    harness.run().await;
    let dropped = harness.cache.calls().iter().any(|call| {
        call.sink == InvalidatedCache::SpendCounter
            && call
                .keys
                .iter()
                .any(|key| key.as_str() == "spend:end_user:unlinked")
    });
    assert!(dropped);
    assert_eq!(
        spend_of(&client, "LiteLLM_EndUserTable", "user_id", "unlinked").await,
        Decimal::ZERO
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn enduser_caches_drop_after_the_non_enduser_ones_and_the_commit() {
    let (fixture, store, client) = schema().await;
    insert_tier(&client, "tier-a", Some("1d"), Some(past())).await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_TeamMembership\" (user_id, team_id, budget_id, spend) VALUES ('user-a', 'team-a', 'tier-a', 10); INSERT INTO \"LiteLLM_EndUserTable\" (user_id, budget_id, spend) VALUES ('customer-a', 'tier-a', 10)",
        )
        .await
        .unwrap();
    let cache = RecordingCache::probing(store.clone());
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        cache,
        CountingReconnect::new(true),
    );
    harness.run().await;
    let calls = harness.cache.calls();
    assert_eq!(calls[0].sink, InvalidatedCache::SpendCounter);
    assert_eq!(
        calls[0].keys,
        vec!["spend:team_member:user-a:team-a".to_owned()]
    );
    let enduser_call = calls
        .iter()
        .position(|call| call.keys == vec!["spend:end_user:customer-a".to_owned()]);
    let position = enduser_call.unwrap();
    assert_eq!(calls[position].sink, InvalidatedCache::SpendCounter);
    assert!(position > 0);
    let seen = harness.cache.seen();
    assert_eq!(seen.len(), 2);
    assert!(seen.iter().all(|spend| *spend == Decimal::ZERO));
    assert_eq!(
        spend_of(&client, "LiteLLM_EndUserTable", "user_id", "customer-a").await,
        Decimal::ZERO
    );
    finish(fixture, store, client).await;
}
