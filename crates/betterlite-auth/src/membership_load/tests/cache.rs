#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_models::decisions::Json;
use serde_json::Number;

use super::support::{Logs, dual_settings, key, lock, membership, stored_membership};
use crate::auth_cache::gateway_cache;
use crate::membership_load::cache::{
    CachedMembership, CachedPayload, MembershipCache, classify_cached,
};

fn classify(cached: CachedPayload) -> CachedMembership {
    classify_cached(cached).expect("the payload classifies")
}

#[test]
fn cache_none_is_miss() {
    assert_eq!(classify(CachedPayload::from(None)), CachedMembership::Miss);
}

#[test]
fn exact_sentinel_is_absence() {
    let sentinel = Json::Text("__no_team_membership__".to_owned());
    assert_eq!(
        classify(CachedPayload::Stored(Box::new(sentinel))),
        CachedMembership::Absent
    );
}

#[test]
fn typed_membership_present() {
    let typed = membership();
    assert_eq!(
        classify(CachedPayload::Typed(Box::new(typed.clone()))),
        CachedMembership::Membership(Box::new(typed))
    );
}

#[test]
fn dictionary_membership_present() {
    assert_eq!(
        classify(CachedPayload::Stored(Box::new(stored_membership()))),
        CachedMembership::Membership(Box::new(membership()))
    );
}

#[test]
fn malformed_dictionary_is_miss() {
    let empty = serde_json::from_str::<Json>("{}").expect("the empty mapping");
    let wrong_user = serde_json::from_str::<Json>(r#"{"user_id":7,"team_id":"team-1"}"#)
        .expect("the invalid membership");
    assert_eq!(
        classify(CachedPayload::Stored(Box::new(empty))),
        CachedMembership::Miss
    );
    assert_eq!(
        classify(CachedPayload::Stored(Box::new(wrong_user))),
        CachedMembership::Miss
    );
}

#[test]
fn scalar_is_miss_without_warning() {
    let (logs, _guard) = Logs::capture();
    let scalars = [
        Json::Null,
        Json::Bool(false),
        Json::Number(Number::from(0_u8)),
        Json::Text("__no_team_membership".to_owned()),
        Json::List(Vec::new()),
    ];
    let misses: Vec<CachedMembership> = scalars
        .into_iter()
        .map(|payload| classify(CachedPayload::Stored(Box::new(payload))))
        .collect();
    assert_eq!(misses, vec![CachedMembership::Miss; 5]);
    assert_eq!(logs.text(), "");
}

#[test]
fn codec_warning_preserved() {
    let (logs, _guard) = Logs::capture();
    let invalid = serde_json::from_str::<Json>(r#"{"user_id":7,"team_id":"team-1"}"#)
        .expect("the invalid membership");
    assert_eq!(
        classify(CachedPayload::Stored(Box::new(invalid))),
        CachedMembership::Miss
    );
    let text = logs.text();
    assert!(
        text.contains("CacheCodec.decode: validation failed"),
        "{text}"
    );
    assert!(text.contains("LiteLLMTeamMembership"), "{text}");
}

#[test]
fn negative_result_differs_from_miss() {
    let sentinel = Json::Text("__no_team_membership__".to_owned());
    let absent = classify(CachedPayload::Stored(Box::new(sentinel)));
    let miss = classify(CachedPayload::Pending);
    assert_eq!(absent, CachedMembership::Absent);
    assert_eq!(miss, CachedMembership::Miss);
    assert_ne!(absent, miss);
}

type SharedClock = Arc<Mutex<f64>>;

// The local tier reads its clock on every write and read, so the test controls expiry.
fn movable_memory(now: &SharedClock) -> Arc<LocalMemoryOps> {
    let reader = Arc::clone(now);
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(move || *lock(&reader));
    Arc::new(LocalMemoryOps::new(1024_i64, clock).expect("the local memory cache builds"))
}

// Reads one key through the cache port and classifies what it holds, as the lookup does.
async fn read_cached<C>(cache: &C, key: &str) -> Option<CachedMembership>
where
    C: MembershipCache,
{
    let stored = cache.read(key, true).await.expect("the cache read")?;
    Some(classify_cached(CachedPayload::from(Some(stored))).expect("the payload classifies"))
}

#[tokio::test]
async fn auth_cache_port_round_trips_and_expires_negative() {
    let now: SharedClock = Arc::new(Mutex::new(1_000.0_f64));
    let cache = gateway_cache(
        movable_memory(&now),
        movable_memory(&now),
        None,
        dual_settings(),
        42.5_f64,
    );
    assert_eq!(cache.management_ttl().to_bits(), 42.5_f64.to_bits());
    let present = key("user-1", "team-1");
    cache
        .write_membership(&present, &membership(), None)
        .await
        .expect("the membership write");
    assert_eq!(
        read_cached(&cache, &present).await,
        Some(CachedMembership::Membership(Box::new(membership())))
    );
    let absent = key("user-2", "team-1");
    cache
        .write_absent(&absent, Some(42.5_f64))
        .await
        .expect("the absence write");
    assert_eq!(
        read_cached(&cache, &absent).await,
        Some(CachedMembership::Absent)
    );
    *lock(&now) = 1_050.0_f64;
    assert!(
        read_cached(&cache, &absent).await.is_none(),
        "the negative entry expires with the management ttl"
    );
    assert!(
        read_cached(&cache, &present).await.is_some(),
        "the positive entry keeps the backend default ttl"
    );
}
