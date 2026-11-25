use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::task::yield_now;
use tokio::time::advance;

use crate::credential_contracts::errors::CredResult;

use super::super::collaborators::ExchangedToken;
use super::super::token_cache::ExchangedTokenCache;
use super::support::{exposed, secret};

async fn minted(
    access_token: &str,
    expires_in: Option<i64>,
    calls: &AtomicUsize,
) -> CredResult<ExchangedToken> {
    yield_now().await;
    calls.fetch_add(1, Ordering::SeqCst);
    let token = ExchangedToken {
        access_token: secret(access_token),
        expires_in,
    };
    Ok(token)
}

async fn fill_range(cache: &ExchangedTokenCache, calls: &AtomicUsize, range: Range<u32>) {
    for index in range {
        let key = format!("key-{index:04}");
        let _stored = cache
            .get_or_compute(key.as_str(), "fp", minted("token", None, calls))
            .await
            .unwrap();
    }
}

async fn yielding_mint(
    access_token: &str,
    calls: &AtomicUsize,
    started: &AtomicUsize,
) -> CredResult<ExchangedToken> {
    started.store(1, Ordering::SeqCst);
    calls.fetch_add(1, Ordering::SeqCst);
    yield_now().await;
    let token = ExchangedToken {
        access_token: secret(access_token),
        expires_in: None,
    };
    Ok(token)
}

async fn wait_started(started: &AtomicUsize) {
    while started.load(Ordering::SeqCst) == 0 {
        yield_now().await;
    }
}

#[tokio::test]
async fn get_or_compute_reuses_the_cached_token_for_the_same_fingerprint() {
    let cache = ExchangedTokenCache::default();
    let calls = AtomicUsize::new(0);
    let first = cache
        .get_or_compute("slot", "fp-1", minted("one", None, &calls))
        .await
        .unwrap();
    let second = cache
        .get_or_compute("slot", "fp-1", minted("two", None, &calls))
        .await
        .unwrap();
    assert_eq!(exposed(&first), "one");
    assert_eq!(exposed(&second), "one");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let third = cache
        .get_or_compute("slot", "fp-2", minted("three", None, &calls))
        .await
        .unwrap();
    assert_eq!(exposed(&third), "three");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn invalidate_removes_the_stored_token() {
    let cache = ExchangedTokenCache::default();
    let calls = AtomicUsize::new(0);
    let first = cache
        .get_or_compute("slot", "fp", minted("one", None, &calls))
        .await
        .unwrap();
    assert_eq!(exposed(&first), "one");
    cache.invalidate("slot");
    let second = cache
        .get_or_compute("slot", "fp", minted("two", None, &calls))
        .await
        .unwrap();
    assert_eq!(exposed(&second), "two");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn an_expired_token_is_recomputed() {
    let cache = ExchangedTokenCache::default();
    let calls = AtomicUsize::new(0);
    let first = cache
        .get_or_compute("slot", "fp", minted("one", Some(120), &calls))
        .await
        .unwrap();
    assert_eq!(exposed(&first), "one");
    advance(Duration::from_secs(59)).await;
    let cached = cache
        .get_or_compute("slot", "fp", minted("two", Some(120), &calls))
        .await
        .unwrap();
    assert_eq!(exposed(&cached), "one");
    advance(Duration::from_secs(1)).await;
    let refreshed = cache
        .get_or_compute("slot", "fp", minted("three", Some(120), &calls))
        .await
        .unwrap();
    assert_eq!(exposed(&refreshed), "three");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn the_ttl_has_a_ten_second_floor() {
    let cache = ExchangedTokenCache::default();
    let calls = AtomicUsize::new(0);
    let first = cache
        .get_or_compute("slot", "fp", minted("one", Some(30), &calls))
        .await
        .unwrap();
    assert_eq!(exposed(&first), "one");
    advance(Duration::from_secs(9)).await;
    let cached = cache
        .get_or_compute("slot", "fp", minted("two", Some(30), &calls))
        .await
        .unwrap();
    assert_eq!(exposed(&cached), "one");
    advance(Duration::from_secs(1)).await;
    let refreshed = cache
        .get_or_compute("slot", "fp", minted("three", Some(30), &calls))
        .await
        .unwrap();
    assert_eq!(exposed(&refreshed), "three");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn the_default_ttl_is_one_hour() {
    let cache = ExchangedTokenCache::default();
    let calls = AtomicUsize::new(0);
    let first = cache
        .get_or_compute("slot", "fp", minted("one", None, &calls))
        .await
        .unwrap();
    assert_eq!(exposed(&first), "one");
    advance(Duration::from_secs(3539)).await;
    let cached = cache
        .get_or_compute("slot", "fp", minted("two", None, &calls))
        .await
        .unwrap();
    assert_eq!(exposed(&cached), "one");
    advance(Duration::from_secs(1)).await;
    let refreshed = cache
        .get_or_compute("slot", "fp", minted("three", None, &calls))
        .await
        .unwrap();
    assert_eq!(exposed(&refreshed), "three");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn invalidate_fences_an_inflight_compute() {
    let cache = ExchangedTokenCache::default();
    let calls = AtomicUsize::new(0);
    let started = AtomicUsize::new(0);
    let (minted_token, ()) = tokio::join!(
        cache.get_or_compute("slot", "fp", yielding_mint("one", &calls, &started)),
        async {
            wait_started(&started).await;
            cache.invalidate("slot");
        }
    );
    assert_eq!(exposed(&minted_token.unwrap()), "one");
    let second = cache
        .get_or_compute("slot", "fp", minted("two", None, &calls))
        .await
        .unwrap();
    assert_eq!(exposed(&second), "two");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_second_key_keeps_the_inflight_guard_alive() {
    let cache = ExchangedTokenCache::default();
    let calls = AtomicUsize::new(0);
    let started = AtomicUsize::new(0);
    let ((), ()) = tokio::join!(
        async {
            let _first = cache
                .get_or_compute("slot-a", "fp", yielding_mint("one", &calls, &started))
                .await
                .unwrap();
        },
        async {
            wait_started(&started).await;
            let _other = cache
                .get_or_compute("slot-b", "fp", minted("other", None, &calls))
                .await
                .unwrap();
            cache.invalidate("slot-a");
        }
    );
    let refreshed = cache
        .get_or_compute("slot-a", "fp", minted("two", None, &calls))
        .await
        .unwrap();
    assert_eq!(exposed(&refreshed), "two");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test(start_paused = true)]
async fn the_cache_evicts_the_earliest_expiry_at_capacity() {
    let cache = ExchangedTokenCache::default();
    let calls = AtomicUsize::new(0);
    fill_range(&cache, &calls, 0..500_u32).await;
    let _overflow = cache
        .get_or_compute("key-extra", "fp", minted("token", None, &calls))
        .await
        .unwrap();
    let _evicted = cache
        .get_or_compute("key-0000", "fp", minted("token", None, &calls))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 502);
    let _kept = cache
        .get_or_compute("key-0200", "fp", minted("token", None, &calls))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 502);
}

#[tokio::test(start_paused = true)]
async fn the_cache_does_not_evict_another_entry_for_a_replacement() {
    let cache = ExchangedTokenCache::default();
    let calls = AtomicUsize::new(0);
    let _long_lived = cache
        .get_or_compute("key-0000", "fp", minted("token", Some(100_000), &calls))
        .await
        .unwrap();
    fill_range(&cache, &calls, 1..500_u32).await;
    let replaced = cache
        .get_or_compute("key-0000", "fp-2", minted("replacement", None, &calls))
        .await
        .unwrap();
    assert_eq!(exposed(&replaced), "replacement");
    let _kept = cache
        .get_or_compute("key-0001", "fp", minted("token", None, &calls))
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 501);
}
