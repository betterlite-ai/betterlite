use tokio::sync::oneshot;
use tokio::task::yield_now;

use super::*;

#[test]
fn the_ttl_subtracts_the_buffer_and_floors_at_the_minimum() {
    let tunables = TokenExchangeTunables::default();
    assert_eq!(
        cache_ttl_seconds(Some(3600), &tunables),
        Duration::from_mins(59)
    );
    assert_eq!(
        cache_ttl_seconds(Some(71), &tunables),
        Duration::from_secs(11)
    );
    assert_eq!(
        cache_ttl_seconds(Some(70), &tunables),
        Duration::from_secs(10)
    );
    assert_eq!(
        cache_ttl_seconds(Some(65), &tunables),
        Duration::from_secs(10)
    );
    assert_eq!(
        cache_ttl_seconds(Some(60), &tunables),
        Duration::from_secs(10)
    );
    assert_eq!(
        cache_ttl_seconds(Some(5), &tunables),
        Duration::from_secs(10)
    );
    assert_eq!(
        cache_ttl_seconds(Some(0), &tunables),
        Duration::from_secs(10)
    );
    assert_eq!(
        cache_ttl_seconds(Some(-5), &tunables),
        Duration::from_secs(10)
    );
    assert_eq!(cache_ttl_seconds(None, &tunables), Duration::from_mins(59));
}

#[test]
fn the_tunables_default_to_the_litellm_constants() {
    let tunables = TokenExchangeTunables::default();
    assert_eq!(tunables.default_ttl, Duration::from_secs(3600));
    assert_eq!(tunables.expiry_buffer, Duration::from_secs(60));
    assert_eq!(tunables.min_ttl, Duration::from_secs(10));
    assert_eq!(tunables.cache_max_size, 500);
}

fn minted(
    computes: Arc<AtomicUsize>,
    token: &'static str,
    expires_in: Option<i64>,
) -> RefreshFuture<'static, ExchangedToken, CredError> {
    Box::pin(async move {
        computes.fetch_add(1, Ordering::SeqCst);
        yield_now().await;
        Ok(ExchangedToken {
            access_token: token.to_owned(),
            expires_in,
        })
    })
}

fn cache() -> (ExchangedTokenCache, Arc<ManualClock>) {
    let clock = ManualClock::new();
    (
        ExchangedTokenCache::new(
            Arc::<ManualClock>::clone(&clock),
            TokenExchangeTunables::default(),
        ),
        clock,
    )
}

#[tokio::test]
async fn a_hit_skips_the_compute() {
    let (cache, _clock) = cache();
    let computes = Arc::new(AtomicUsize::new(0));
    let first = cache
        .get_or_compute(
            "key",
            "",
            minted(Arc::clone(&computes), "token-a", Some(600)),
        )
        .await
        .unwrap();
    let second = cache
        .get_or_compute(
            "key",
            "",
            minted(Arc::clone(&computes), "token-b", Some(600)),
        )
        .await
        .unwrap();
    assert_eq!(first, "token-a");
    assert_eq!(second, "token-a");
    assert_eq!(computes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_fingerprint_mismatch_recomputes() {
    let (cache, _clock) = cache();
    let computes = Arc::new(AtomicUsize::new(0));
    cache
        .get_or_compute(
            "key",
            "user-a",
            minted(Arc::clone(&computes), "token-a", Some(600)),
        )
        .await
        .unwrap();
    let other = cache
        .get_or_compute(
            "key",
            "user-b",
            minted(Arc::clone(&computes), "token-b", Some(600)),
        )
        .await
        .unwrap();
    let again = cache
        .get_or_compute(
            "key",
            "user-b",
            minted(Arc::clone(&computes), "token-c", Some(600)),
        )
        .await
        .unwrap();
    assert_eq!(other, "token-b");
    assert_eq!(again, "token-b");
    assert_eq!(computes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_remint_stores_the_new_tokens_own_ttl() {
    let (cache, clock) = cache();
    let computes = Arc::new(AtomicUsize::new(0));
    cache
        .get_or_compute(
            "key",
            "user-a",
            minted(Arc::clone(&computes), "token-a", Some(600)),
        )
        .await
        .unwrap();
    clock.advance(100);
    assert_eq!(
        cache
            .get_or_compute(
                "key",
                "user-b",
                minted(Arc::clone(&computes), "token-b", Some(65))
            )
            .await
            .unwrap(),
        "token-b"
    );
    clock.advance(9);
    assert_eq!(
        cache
            .get_or_compute(
                "key",
                "user-b",
                minted(Arc::clone(&computes), "unused", None)
            )
            .await
            .unwrap(),
        "token-b"
    );
    clock.advance(1);
    assert_eq!(
        cache
            .get_or_compute(
                "key",
                "user-b",
                minted(Arc::clone(&computes), "token-c", None)
            )
            .await
            .unwrap(),
        "token-c"
    );
    assert_eq!(computes.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn concurrent_misses_on_one_key_share_a_compute() {
    let (cache, _clock) = cache();
    let computes = Arc::new(AtomicUsize::new(0));
    let (first, second) = tokio::join!(
        cache.get_or_compute(
            "key",
            "",
            minted(Arc::clone(&computes), "token-a", Some(600))
        ),
        cache.get_or_compute(
            "key",
            "",
            minted(Arc::clone(&computes), "token-b", Some(600))
        ),
    );
    assert_eq!(first.unwrap(), "token-a");
    assert_eq!(second.unwrap(), "token-a");
    assert_eq!(computes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn invalidate_forces_the_next_compute() {
    let (cache, _clock) = cache();
    let computes = Arc::new(AtomicUsize::new(0));
    cache
        .get_or_compute(
            "key",
            "",
            minted(Arc::clone(&computes), "token-a", Some(600)),
        )
        .await
        .unwrap();
    cache.invalidate("key");
    let second = cache
        .get_or_compute(
            "key",
            "",
            minted(Arc::clone(&computes), "token-b", Some(600)),
        )
        .await
        .unwrap();
    assert_eq!(second, "token-b");
    assert_eq!(computes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn invalidate_during_a_compute_wins_over_its_write() {
    let (cache, _clock) = cache();
    let (started_sender, started) = oneshot::channel::<()>();
    let (release, released) = oneshot::channel::<()>();
    let flight = cache.get_or_compute(
        "key",
        "",
        Box::pin(async move {
            started_sender.send(()).unwrap();
            released.await.unwrap();
            Ok(ExchangedToken {
                access_token: "stale".to_owned(),
                expires_in: Some(600),
            })
        }),
    );
    let evict = async {
        started.await.unwrap();
        cache.invalidate("key");
        release.send(()).unwrap();
    };
    let (outcome, ()) = tokio::join!(flight, evict);
    assert_eq!(outcome.unwrap(), "stale");
    let computes = Arc::new(AtomicUsize::new(0));
    let fresh = cache
        .get_or_compute("key", "", minted(Arc::clone(&computes), "fresh", Some(600)))
        .await
        .unwrap();
    assert_eq!(fresh, "fresh");
    assert_eq!(computes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_failed_compute_is_not_stored() {
    let (cache, _clock) = cache();
    let failure = cache
        .get_or_compute(
            "key",
            "",
            Box::pin(async { Err(CredError::UpstreamUnavailable("endpoint down".to_owned())) }),
        )
        .await;
    assert_eq!(
        failure,
        Err(CredError::UpstreamUnavailable("endpoint down".to_owned()))
    );
    let computes = Arc::new(AtomicUsize::new(0));
    let second = cache
        .get_or_compute(
            "key",
            "",
            minted(Arc::clone(&computes), "token-a", Some(600)),
        )
        .await
        .unwrap();
    let third = cache
        .get_or_compute(
            "key",
            "",
            minted(Arc::clone(&computes), "token-b", Some(600)),
        )
        .await
        .unwrap();
    assert_eq!(second, "token-a");
    assert_eq!(third, "token-a");
    assert_eq!(computes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_entry_expires_at_the_ttl_after_the_buffer() {
    let (cache, clock) = cache();
    let computes = Arc::new(AtomicUsize::new(0));
    cache
        .get_or_compute(
            "key",
            "",
            minted(Arc::clone(&computes), "token-a", Some(65)),
        )
        .await
        .unwrap();
    clock.advance(9);
    assert_eq!(
        cache
            .get_or_compute("key", "", minted(Arc::clone(&computes), "unused", None))
            .await
            .unwrap(),
        "token-a"
    );
    clock.advance(1);
    assert_eq!(
        cache
            .get_or_compute("key", "", minted(Arc::clone(&computes), "token-b", None))
            .await
            .unwrap(),
        "token-b"
    );
    assert_eq!(computes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_missing_expiry_uses_the_default_ttl() {
    let (cache, clock) = cache();
    let computes = Arc::new(AtomicUsize::new(0));
    cache
        .get_or_compute("key", "", minted(Arc::clone(&computes), "token-a", None))
        .await
        .unwrap();
    clock.advance(3539);
    assert_eq!(
        cache
            .get_or_compute("key", "", minted(Arc::clone(&computes), "token-b", None))
            .await
            .unwrap(),
        "token-a"
    );
    clock.advance(1);
    assert_eq!(
        cache
            .get_or_compute("key", "", minted(Arc::clone(&computes), "token-c", None))
            .await
            .unwrap(),
        "token-c"
    );
    assert_eq!(computes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn the_cache_evicts_at_capacity() {
    let clock = ManualClock::new();
    let tunables = TokenExchangeTunables {
        cache_max_size: 1,
        default_ttl: Duration::from_secs(3600),
        min_ttl: Duration::from_secs(10),
        expiry_buffer: Duration::from_secs(60),
    };
    let cache = ExchangedTokenCache::new(clock, tunables);
    let computes = Arc::new(AtomicUsize::new(0));
    cache
        .get_or_compute(
            "first",
            "",
            minted(Arc::clone(&computes), "token-a", Some(600)),
        )
        .await
        .unwrap();
    cache
        .get_or_compute(
            "second",
            "",
            minted(Arc::clone(&computes), "token-b", Some(600)),
        )
        .await
        .unwrap();
    let first = cache
        .get_or_compute(
            "first",
            "",
            minted(Arc::clone(&computes), "token-c", Some(600)),
        )
        .await
        .unwrap();
    assert_eq!(first, "token-c");
    assert_eq!(computes.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn the_cache_evicts_the_entry_that_expires_soonest() {
    let clock = ManualClock::new();
    let tunables = TokenExchangeTunables {
        cache_max_size: 2,
        default_ttl: Duration::from_secs(3600),
        min_ttl: Duration::from_secs(10),
        expiry_buffer: Duration::from_secs(60),
    };
    let cache = ExchangedTokenCache::new(clock, tunables);
    let computes = Arc::new(AtomicUsize::new(0));
    cache
        .get_or_compute(
            "long",
            "",
            minted(Arc::clone(&computes), "token-a", Some(3600)),
        )
        .await
        .unwrap();
    cache
        .get_or_compute(
            "short",
            "",
            minted(Arc::clone(&computes), "token-b", Some(65)),
        )
        .await
        .unwrap();
    cache
        .get_or_compute(
            "third",
            "",
            minted(Arc::clone(&computes), "token-c", Some(3600)),
        )
        .await
        .unwrap();
    let long = cache
        .get_or_compute("long", "", minted(Arc::clone(&computes), "unused", None))
        .await
        .unwrap();
    let short = cache
        .get_or_compute(
            "short",
            "",
            minted(Arc::clone(&computes), "token-d", Some(3600)),
        )
        .await
        .unwrap();
    assert_eq!(long, "token-a");
    assert_eq!(short, "token-d");
    assert_eq!(computes.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn a_zero_max_size_caches_nothing() {
    let clock = ManualClock::new();
    let tunables = TokenExchangeTunables {
        cache_max_size: 0,
        default_ttl: Duration::from_secs(3600),
        min_ttl: Duration::from_secs(10),
        expiry_buffer: Duration::from_secs(60),
    };
    let cache = ExchangedTokenCache::new(clock, tunables);
    let computes = Arc::new(AtomicUsize::new(0));
    let first = cache
        .get_or_compute(
            "key",
            "",
            minted(Arc::clone(&computes), "token-a", Some(600)),
        )
        .await
        .unwrap();
    let second = cache
        .get_or_compute(
            "key",
            "",
            minted(Arc::clone(&computes), "token-b", Some(600)),
        )
        .await
        .unwrap();
    assert_eq!(first, "token-a");
    assert_eq!(second, "token-b");
    assert_eq!(computes.load(Ordering::SeqCst), 2);
}
