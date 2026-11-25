#![cfg(test)]

use std::collections::HashMap;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use crate::gateway_dcr::oauth_error::{OauthError, OauthErrorCode};
use crate::gateway_dcr::single_use::{
    CLAIM_UNAVAILABLE_DESCRIPTION, ClaimAuthority, ClaimBackend, ClaimBackendError, ClaimClock,
    ClaimOutcome, MAX_MEMORY_CLAIMS, MemoryClaimStore, PeekOutcome, SingleUseGuard,
    SystemClaimClock, claim_refusal,
};

const KEY: &str = "mcp_gateway_dcr_code_used:jti-1";
const TTL: i64 = 120;

struct FixedClock(Arc<AtomicI64>);

impl ClaimClock for FixedClock {
    fn now_seconds(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn clock_arc(seconds: i64) -> Arc<AtomicI64> {
    Arc::new(AtomicI64::new(seconds))
}

fn local_store(seconds: i64) -> (MemoryClaimStore, Arc<AtomicI64>) {
    let ticker = clock_arc(seconds);
    let store = MemoryClaimStore {
        clock: Box::new(FixedClock(Arc::clone(&ticker))),
        entries: Mutex::new(HashMap::new()),
    };
    (store, ticker)
}

fn entry_count(store: &MemoryClaimStore) -> usize {
    store.entries.lock().unwrap().len()
}

#[derive(Default)]
struct CountingBackend {
    counts: Mutex<HashMap<String, u64>>,
}

impl ClaimBackend for CountingBackend {
    fn increment(
        &self,
        key: &str,
        _ttl_seconds: i64,
    ) -> impl Future<Output = Result<u64, ClaimBackendError>> + Send {
        let mut counts = self.counts.lock().unwrap();
        let entry = counts.entry(key.to_owned()).or_insert(0);
        *entry = entry.saturating_add(1);
        let total = *entry;
        drop(counts);
        ready(Ok(total))
    }

    fn read(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<u64>, ClaimBackendError>> + Send {
        ready(Ok(self.counts.lock().unwrap().get(key).copied()))
    }
}

struct FixedCount(u64);

impl ClaimBackend for FixedCount {
    fn increment(
        &self,
        _key: &str,
        _ttl_seconds: i64,
    ) -> impl Future<Output = Result<u64, ClaimBackendError>> + Send {
        ready(Ok(self.0))
    }

    fn read(
        &self,
        _key: &str,
    ) -> impl Future<Output = Result<Option<u64>, ClaimBackendError>> + Send {
        ready(Ok(Some(self.0)))
    }
}

struct FaultyBackend;

impl ClaimBackend for FaultyBackend {
    fn increment(
        &self,
        _key: &str,
        _ttl_seconds: i64,
    ) -> impl Future<Output = Result<u64, ClaimBackendError>> + Send {
        ready(Err(ClaimBackendError))
    }

    fn read(
        &self,
        _key: &str,
    ) -> impl Future<Output = Result<Option<u64>, ClaimBackendError>> + Send {
        ready(Err(ClaimBackendError))
    }
}

fn replayed_error() -> OauthError {
    OauthError {
        status: 400,
        error: OauthErrorCode::InvalidGrant,
        description: "the authorization code was already used".to_owned(),
    }
}

#[tokio::test]
async fn local_first_claim_is_first_and_the_second_is_replayed() {
    let (store, _ticker) = local_store(1_000);
    let guard = SingleUseGuard::<CountingBackend>::from(ClaimAuthority::Local(store));
    assert_eq!(guard.claim(KEY, TTL).await, ClaimOutcome::First);
    assert_eq!(guard.claim(KEY, TTL).await, ClaimOutcome::Replayed);
    assert_eq!(guard.claim(KEY, TTL).await, ClaimOutcome::Replayed);
}

#[tokio::test]
async fn local_peek_never_creates_the_key() {
    let (store, _ticker) = local_store(1_000);
    let guard = SingleUseGuard::<CountingBackend>::from(ClaimAuthority::Local(store));
    assert_eq!(guard.peek(KEY).await, PeekOutcome::Unclaimed);
    assert_eq!(guard.claim(KEY, TTL).await, ClaimOutcome::First);
    assert_eq!(guard.peek(KEY).await, PeekOutcome::Claimed);
    assert_eq!(guard.claim(KEY, TTL).await, ClaimOutcome::Replayed);
}

#[tokio::test]
async fn local_store_keeps_the_original_expiry_across_increments() {
    let (store, ticker) = local_store(1_000);
    assert_eq!(store.increment(KEY, TTL), 1);
    ticker.store(1_059, Ordering::SeqCst);
    assert_eq!(store.increment(KEY, TTL), 2);
    ticker.store(1_119, Ordering::SeqCst);
    assert_eq!(store.increment(KEY, TTL), 3);
    ticker.store(1_120, Ordering::SeqCst);
    assert_eq!(store.increment(KEY, TTL), 1);
    ticker.store(1_239, Ordering::SeqCst);
    assert_eq!(store.increment(KEY, TTL), 2);
    ticker.store(1_240, Ordering::SeqCst);
    assert_eq!(store.increment(KEY, TTL), 1);
}

#[tokio::test]
async fn local_read_returns_none_after_expiry() {
    let (store, ticker) = local_store(1_000);
    assert_eq!(store.increment(KEY, TTL), 1);
    assert_eq!(store.read(KEY), Some(1));
    ticker.store(1_119, Ordering::SeqCst);
    assert_eq!(store.read(KEY), Some(1));
    ticker.store(1_120, Ordering::SeqCst);
    assert_eq!(store.read(KEY), None);
}

#[tokio::test]
async fn local_claims_are_independent_per_key() {
    let (store, _ticker) = local_store(1_000);
    let guard = SingleUseGuard::<CountingBackend>::from(ClaimAuthority::Local(store));
    assert_eq!(guard.claim(KEY, TTL).await, ClaimOutcome::First);
    assert_eq!(
        guard.peek("mcp_gateway_dcr_flow_used:jti-1").await,
        PeekOutcome::Unclaimed
    );
    assert_eq!(
        guard.claim("mcp_gateway_dcr_flow_used:jti-1", TTL).await,
        ClaimOutcome::First
    );
}

#[tokio::test]
async fn local_store_keeps_live_claims_and_sweeps_expired_ones_at_capacity() {
    let (store, ticker) = local_store(1_000);
    (0..MAX_MEMORY_CLAIMS).for_each(|index| {
        assert_eq!(store.increment(&format!("key-{index}"), 10), 1);
    });
    assert_eq!(entry_count(&store), MAX_MEMORY_CLAIMS);
    assert_eq!(store.increment("extra", 10), 1);
    assert_eq!(entry_count(&store), MAX_MEMORY_CLAIMS.saturating_add(1));
    assert_eq!(store.read("key-0"), Some(1));

    // the deadline itself is already expired, so the sweep drops every key at exactly 1_010
    ticker.store(1_010, Ordering::SeqCst);
    assert_eq!(store.increment("fresh", 10), 1);
    assert_eq!(entry_count(&store), 1);
    assert_eq!(store.read("key-0"), None);
    assert_eq!(store.read("fresh"), Some(1));
}

#[tokio::test]
async fn shared_first_claim_is_first_and_the_second_is_replayed() {
    let guard = SingleUseGuard::from(ClaimAuthority::Shared(CountingBackend::default()));
    assert_eq!(guard.claim(KEY, TTL).await, ClaimOutcome::First);
    assert_eq!(guard.claim(KEY, TTL).await, ClaimOutcome::Replayed);
}

#[tokio::test]
async fn a_shared_count_other_than_one_is_replayed() {
    let zero = SingleUseGuard::from(ClaimAuthority::Shared(FixedCount(0)));
    assert_eq!(zero.claim(KEY, TTL).await, ClaimOutcome::Replayed);
    let two = SingleUseGuard::from(ClaimAuthority::Shared(FixedCount(2)));
    assert_eq!(two.claim(KEY, TTL).await, ClaimOutcome::Replayed);
}

#[tokio::test]
async fn shared_peek_reads_unclaimed_claimed_and_unavailable() {
    let guard = SingleUseGuard::from(ClaimAuthority::Shared(CountingBackend::default()));
    assert_eq!(guard.peek(KEY).await, PeekOutcome::Unclaimed);
    assert_eq!(guard.claim(KEY, TTL).await, ClaimOutcome::First);
    assert_eq!(guard.peek(KEY).await, PeekOutcome::Claimed);
    let faulty = SingleUseGuard::from(ClaimAuthority::Shared(FaultyBackend));
    assert_eq!(faulty.peek(KEY).await, PeekOutcome::Unavailable);
}

#[tokio::test]
async fn a_shared_backend_error_fails_closed_without_falling_back_to_local() {
    let (store, _ticker) = local_store(1_000);
    assert_eq!(store.increment(KEY, TTL), 1);
    let guard = SingleUseGuard::from(ClaimAuthority::Shared(FaultyBackend));
    assert_eq!(guard.claim(KEY, TTL).await, ClaimOutcome::Unavailable);
    assert_eq!(guard.peek(KEY).await, PeekOutcome::Unavailable);
    assert_eq!(store.read(KEY), Some(1));
}

#[tokio::test]
async fn a_shared_backend_decision_ignores_the_local_store() {
    let (store, _ticker) = local_store(1_000);
    assert_eq!(store.increment(KEY, TTL), 1);
    let guard = SingleUseGuard::from(ClaimAuthority::Shared(FixedCount(1)));
    assert_eq!(guard.claim(KEY, TTL).await, ClaimOutcome::First);
}

#[test]
fn claim_refusal_passes_the_first_claim() {
    assert_eq!(claim_refusal(ClaimOutcome::First, replayed_error()), None);
}

#[test]
fn claim_refusal_returns_the_replayed_error() {
    let refusal = claim_refusal(ClaimOutcome::Replayed, replayed_error()).unwrap();
    assert_eq!(refusal, replayed_error());
}

#[test]
fn claim_refusal_answers_503_when_the_claim_could_not_be_recorded() {
    let refusal = claim_refusal(ClaimOutcome::Unavailable, replayed_error()).unwrap();
    assert_eq!(refusal.status, 503);
    assert_eq!(refusal.error, OauthErrorCode::TemporarilyUnavailable);
    assert_eq!(
        refusal.description,
        "the single-use record is unavailable right now; try again shortly"
    );
    assert_eq!(refusal.description, CLAIM_UNAVAILABLE_DESCRIPTION);
}

#[test]
fn system_claim_clock_reads_the_wall_clock() {
    assert!(SystemClaimClock.now_seconds() > 1_700_000_000);
}
