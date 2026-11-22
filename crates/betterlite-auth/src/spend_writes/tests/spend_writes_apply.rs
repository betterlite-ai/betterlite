use std::future::pending;
use std::str::from_utf8;
use std::sync::Arc;
use std::time::Duration;

use betterlite_cache::backend::BackendError;
use betterlite_redis::cache_primitives::decode::DecodeError;
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use betterlite_redis::redis_client_reuse::breaker::BreakerOpen;
use betterlite_redis::redis_client_reuse::error::ClientReuseError;
use betterlite_redis::redis_counters::RedisCountersError;
use betterlite_redis::redis_live_client::LiveClientError;
use rust_decimal::Decimal;
use tokio::time::timeout;

use crate::spend_counter_batch::batch::SpendCounterBatchError;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::reseed::CoalescedError;
use crate::spend_reseed::values::SpendValueError;
use crate::spend_writes::error::{LayersError, SpendWriteError};

use super::support::{
    Failure, FakePostCall, Fault, KEY, breaker_redis_error, decimal, fixture, increment,
    timed_out_live, timeout_redis_error,
};

#[tokio::test]
async fn apply_uses_the_post_call_batch_when_one_is_served() {
    let fx = fixture(true);
    let post = Arc::new(FakePostCall::new(60, fx.journal.clone()));
    fx.layers.serve_post_call(Arc::clone(&post));
    fx.writes
        .apply_spend_counter_increments(&[increment(KEY, "2")])
        .await
        .expect("the deferred apply succeeds");
    assert_eq!(post.queued(), vec![(KEY.to_owned(), 2.0_f64, 60_i64)]);
    assert_eq!(fx.journal.count("pipeline:"), 0);
    assert_eq!(fx.layers.increment_total(KEY), None);
}

#[tokio::test]
async fn apply_runs_the_validating_pipeline_without_a_post_call_batch() {
    let fx = fixture(true);
    fx.writes
        .apply_spend_counter_increments(&[increment(KEY, "2")])
        .await
        .expect("the pipeline applies the increment");
    assert_eq!(fx.layers.increment_total(KEY), Some(decimal("2")));
    assert!(fx.journal.records("pipeline:1"));
}

#[tokio::test]
async fn apply_swallows_a_breaker_failure() {
    let fx = fixture(true);
    fx.layers.faults.add(Fault::Pipeline);
    fx.writes
        .apply_spend_counter_increments(&[increment(KEY, "2")])
        .await
        .expect("the breaker failure settles itself");
    assert_eq!(fx.journal.count("batch_forget:"), 1);
}

#[tokio::test]
async fn apply_swallows_a_timeout_failure() {
    let fx = fixture(true);
    fx.layers.faults.add(Fault::Pipeline);
    *fx.layers.failure.lock().expect("the fake failure lock") = Failure::Timeout;
    fx.writes
        .apply_spend_counter_increments(&[increment(KEY, "2")])
        .await
        .expect("the timeout settles itself");
    assert_eq!(fx.journal.count("batch_forget:"), 1);
}

#[tokio::test]
async fn apply_propagates_a_non_transient_failure() {
    let fx = fixture(true);
    fx.layers.faults.add(Fault::Pipeline);
    *fx.layers.failure.lock().expect("the fake failure lock") = Failure::Unavailable;
    let error = fx
        .writes
        .apply_spend_counter_increments(&[increment(KEY, "2")])
        .await
        .expect_err("the missing tier surfaces");
    assert!(matches!(
        error,
        SpendWriteError::Layers(LayersError::Unavailable)
    ));
    assert!(!error.is_transient());
}

#[tokio::test]
async fn apply_with_nothing_pending_succeeds_silently() {
    let fx = fixture(true);
    fx.writes
        .apply_spend_counter_increments(&[])
        .await
        .expect("an empty apply succeeds");
    assert!(fx.journal.events().is_empty());
}

#[test]
fn transient_errors_are_the_ones_redis_clears_on_its_own() {
    assert!(LayersError::Redis(breaker_redis_error()).is_transient());
    assert!(LayersError::Redis(timeout_redis_error()).is_transient());
    assert!(!LayersError::Unavailable.is_transient());
    assert!(!LayersError::Memory(BackendError::Unsupported("a tier")).is_transient());
    assert!(!LayersError::NonFinite(f64::INFINITY).is_transient());
    assert!(
        !LayersError::Admin(RedisAdminError::Breaker(BreakerOpen { name: "admin" })).is_transient()
    );

    assert!(SpendWriteError::Layers(LayersError::Redis(breaker_redis_error())).is_transient());
    assert!(!SpendWriteError::Layers(LayersError::Unavailable).is_transient());
    assert!(SpendWriteError::Cache(ReseedCacheError::Redis(breaker_redis_error())).is_transient());
    assert!(!SpendWriteError::Cache(ReseedCacheError::Unavailable).is_transient());
    assert!(
        !SpendWriteError::Cache(ReseedCacheError::Memory(BackendError::Unsupported(
            "a tier"
        )))
        .is_transient()
    );
    assert!(!SpendWriteError::Cache(ReseedCacheError::OutOfRange(Decimal::ONE)).is_transient());
    assert!(
        !SpendWriteError::Cache(ReseedCacheError::Values(
            RedisCacheValuesError::TtlOutOfRange { seconds: 1.0 }
        ))
        .is_transient()
    );
    assert!(!SpendWriteError::Batch(SpendCounterBatchError::Cancelled).is_transient());
    assert!(
        !SpendWriteError::Reseed(CoalescedError::Cache(ReseedCacheError::Unavailable))
            .is_transient()
    );
    assert!(
        !SpendWriteError::Value(SpendValueError::NotNumeric { kind: "a value" }).is_transient()
    );
}

fn utf8_decode_error() -> DecodeError {
    let mut bytes = vec![b'a'];
    bytes.push(0xff);
    let source = from_utf8(&bytes).expect_err("the bytes are not utf-8");
    DecodeError::Utf8 { source }
}

#[test]
fn every_redis_error_classification_is_reached() {
    let decoded = LayersError::Redis(RedisCacheError::Decode(utf8_decode_error()));
    assert!(!decoded.is_transient());

    let invalid_ttl = LayersError::Redis(RedisCacheError::Counters(RedisCountersError::InvalidTtl));
    assert!(!invalid_ttl.is_transient());
    let no_finite = LayersError::Redis(RedisCacheError::Counters(
        RedisCountersError::NonFiniteValue,
    ));
    assert!(!no_finite.is_transient());
    let counters_breaker = LayersError::Redis(RedisCacheError::Counters(
        RedisCountersError::Breaker(BreakerOpen { name: "counters" }),
    ));
    assert!(counters_breaker.is_transient());
    let counters_live = LayersError::Redis(RedisCacheError::Counters(RedisCountersError::Client(
        timed_out_live(),
    )));
    assert!(counters_live.is_transient());
    let counters_reuse = LayersError::Redis(RedisCacheError::Counters(RedisCountersError::Reuse(
        ClientReuseError::Live(timed_out_live()),
    )));
    assert!(counters_reuse.is_transient());

    let no_pubsub =
        LayersError::Redis(RedisCacheError::ClientReuse(ClientReuseError::NoPubSubNode));
    assert!(!no_pubsub.is_transient());
    let reuse_live = LayersError::Redis(RedisCacheError::ClientReuse(ClientReuseError::Live(
        timed_out_live(),
    )));
    assert!(reuse_live.is_transient());

    let live_timeout = LayersError::Redis(timeout_redis_error());
    assert!(live_timeout.is_transient());
    let no_source = LayersError::Redis(RedisCacheError::LiveClient(
        LiveClientError::CredentialSourceMissing,
    ));
    assert!(!no_source.is_transient());
}

#[tokio::test]
async fn a_connect_timeout_is_transient() {
    let elapsed = timeout(Duration::ZERO, pending::<()>())
        .await
        .expect_err("the timeout expires");
    let error = LayersError::Redis(RedisCacheError::ClientReuse(ClientReuseError::Timeout(
        elapsed,
    )));
    assert!(error.is_transient());
}
