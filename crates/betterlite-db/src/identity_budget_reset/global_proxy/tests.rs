#![cfg(test)]

use rust_decimal::Decimal;

use super::{PROXY_BUDGET_USER_ID, accumulator_cache_keys};
use crate::budget_reset_policy::cache_invalidation::{
    GLOBAL_PROXY_SPEND_CACHE_KEY, InvalidatedCache,
};
use crate::identity_budget_reset::phase::IdentityPhase;
use crate::identity_budget_reset::tests::support::{
    CountingReconnect, Harness, RecordingCache, TestSchedule, finish, moment, schema, write,
};

#[test]
fn global_proxy_names_the_litellm_budget_user_and_accumulator() {
    assert_eq!(PROXY_BUDGET_USER_ID, "litellm-proxy-budget");
    assert_eq!(GLOBAL_PROXY_SPEND_CACHE_KEY, "default_user_id:spend");
}

#[test]
fn global_proxy_drops_the_accumulator_only_for_the_budget_user() {
    let proxy = vec![write(PROXY_BUDGET_USER_ID, None, Decimal::ZERO)];
    assert_eq!(
        accumulator_cache_keys(IdentityPhase::Users, &proxy),
        vec![GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned()]
    );
    let ordinary = vec![write("someone-else", None, Decimal::ZERO)];
    assert!(accumulator_cache_keys(IdentityPhase::Users, &ordinary).is_empty());
    assert!(accumulator_cache_keys(IdentityPhase::Keys, &proxy).is_empty());
    assert!(accumulator_cache_keys(IdentityPhase::Keys, &ordinary).is_empty());
}

#[tokio::test]
async fn global_proxy_invalidates_the_accumulator_after_the_spend_counter() {
    let (fixture, store, client) = schema().await;
    client
        .batch_execute(
            "INSERT INTO \"LiteLLM_UserTable\" (user_id, spend, budget_duration, budget_reset_at) VALUES ('litellm-proxy-budget', 10, '1d', NULL);",
        )
        .await
        .unwrap();
    let mut harness = Harness::new(
        store.clone(),
        TestSchedule::fixed(moment(30)),
        RecordingCache::default(),
        CountingReconnect::new(true),
    );
    harness.run(IdentityPhase::Users).await;
    let calls = harness.cache.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].sink, InvalidatedCache::SpendCounter);
    assert_eq!(
        calls[0].keys,
        vec!["spend:user:litellm-proxy-budget".to_owned()]
    );
    assert_eq!(calls[1].sink, InvalidatedCache::UserApiKeyCache);
    assert_eq!(calls[1].keys, vec![GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned()]);
    finish(fixture, store, client).await;
}
