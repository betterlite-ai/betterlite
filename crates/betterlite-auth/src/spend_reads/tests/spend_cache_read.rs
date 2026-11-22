use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_cache::backend::memory::MemoryCacheOps as _;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};

use super::support::{Captured, Fixture, KEY, decimal};
use crate::spend_counter_batch::batch::CounterValue;
use crate::spend_counter_batch::scope::SpendCounterScope;
use crate::spend_reads::SpendReads;
use crate::spend_reads::cache::SpendReadCache as _;
use crate::spend_reseed::cache::ReseedCache as _;
use crate::spend_reseed::values::ReseedCacheValue;

const FRESH_KEY: &str = "spend:key:fresh";

#[tokio::test]
async fn a_batched_amount_answers_before_redis() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("5")));
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("3")));
    fixture
        .batch
        .set_answer(Some(CounterValue::Amount(decimal("4"))));

    let answer = fixture
        .reads(false)
        .read_spend_counter_cache_value(KEY)
        .await;

    assert_eq!(answer, (Some(decimal("4")), true));
    assert_eq!(fixture.events(), [format!("batch_read:{KEY}")]);
}

#[tokio::test]
async fn a_batched_miss_is_authoritative_and_skips_memory() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("3")));
    fixture.batch.set_answer(Some(CounterValue::Missing));

    let answer = fixture
        .reads(false)
        .read_spend_counter_cache_value(KEY)
        .await;

    assert_eq!(answer, (None, true));
    assert_eq!(fixture.events(), [format!("batch_read:{KEY}")]);
}

#[tokio::test]
async fn a_counter_outside_the_batch_falls_through_to_redis() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("5")));

    let answer = fixture
        .reads(false)
        .read_spend_counter_cache_value(KEY)
        .await;

    assert_eq!(answer, (Some(decimal("5")), true));
    assert_eq!(
        fixture.events(),
        [format!("batch_read:{KEY}"), format!("redis_get:{KEY}")]
    );
}

#[tokio::test]
async fn a_failed_batch_read_falls_through_to_redis() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("5")));
    fixture.batch.fail();

    let answer = fixture
        .reads(false)
        .read_spend_counter_cache_value(KEY)
        .await;

    assert_eq!(answer, (Some(decimal("5")), true));
    assert!(logs.has("the spend counter batch read failed; reading Redis directly"));
}

#[tokio::test]
async fn redis_answers_with_cross_pod_authority() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("5")));
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("3")));

    let answer = fixture
        .reads(false)
        .read_spend_counter_cache_value(KEY)
        .await;

    assert_eq!(answer, (Some(decimal("5")), true));
    assert!(
        !fixture
            .events()
            .iter()
            .any(|event| event.starts_with("memory_get:"))
    );
}

#[tokio::test]
async fn a_clean_redis_miss_answers_an_authoritative_absence() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("3")));

    let answer = fixture
        .reads(false)
        .read_spend_counter_cache_value(KEY)
        .await;

    assert_eq!(answer, (None, true));
    assert!(
        !fixture
            .events()
            .iter()
            .any(|event| event.starts_with("memory_get:"))
    );
}

#[tokio::test]
async fn a_text_redis_value_reads_as_spend() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Text("2.5".to_owned()));

    let answer = fixture
        .reads(false)
        .read_spend_counter_cache_value(KEY)
        .await;

    assert_eq!(answer, (Some(decimal("2.5")), true));
}

#[tokio::test]
async fn an_unreadable_redis_value_falls_back_to_memory() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Text("abc".to_owned()));
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("3")));

    let answer = fixture
        .reads(false)
        .read_spend_counter_cache_value(KEY)
        .await;

    assert_eq!(answer, (Some(decimal("3")), false));
    assert!(logs.has("the spend counter value is not a number"));
}

#[tokio::test]
async fn a_failed_redis_read_falls_back_to_memory() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("3")));
    fixture.cache.fail_redis();

    let answer = fixture
        .reads(false)
        .read_spend_counter_cache_value(KEY)
        .await;

    assert_eq!(answer, (Some(decimal("3")), false));
    assert!(logs.has("the Redis spend counter read failed; falling back to memory"));
}

#[tokio::test]
async fn memory_answers_only_with_a_per_pod_estimate() {
    let fixture = Fixture::new(false);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Number(decimal("3")));

    let answer = fixture
        .reads(false)
        .read_spend_counter_cache_value(KEY)
        .await;

    assert_eq!(answer, (Some(decimal("3")), false));
}

#[tokio::test]
async fn an_unreadable_memory_value_reads_as_absent() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(false);
    fixture
        .cache
        .seed_memory(KEY, ReseedCacheValue::Text("abc".to_owned()));

    let answer = fixture
        .reads(false)
        .read_spend_counter_cache_value(KEY)
        .await;

    assert_eq!(answer, (None, false));
    assert!(logs.has("the spend counter value is not a number"));
}

#[tokio::test]
async fn a_failed_memory_read_reads_as_absent() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(false);
    fixture.cache.fail_memory_reads();

    let answer = fixture
        .reads(false)
        .read_spend_counter_cache_value(KEY)
        .await;

    assert_eq!(answer, (None, false));
    assert!(logs.has("the in-memory spend counter read failed"));
}

#[tokio::test]
async fn nothing_anywhere_answers_a_clean_absence() {
    let fixture = Fixture::new(true);

    let answer = fixture
        .reads(false)
        .read_spend_counter_cache_value(KEY)
        .await;

    assert_eq!(answer, (None, true));
}

#[tokio::test]
async fn the_batch_step_is_skipped_without_a_request_batch() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("5")));

    let answer = fixture
        .reads_without_batch(false)
        .read_spend_counter_cache_value(KEY)
        .await;

    assert_eq!(answer, (Some(decimal("5")), true));
    assert!(
        !fixture
            .events()
            .iter()
            .any(|event| event.starts_with("batch_read:"))
    );
}

#[tokio::test]
async fn the_request_scope_batch_adapter_reads_through_its_absent_batch() {
    let fixture = Fixture::new(true);
    fixture
        .cache
        .seed_redis(KEY, ReseedCacheValue::Number(decimal("5")));
    let scope = SpendCounterScope::default();
    let reads = SpendReads {
        reseed: &fixture.reseed,
        cache: fixture.cache.as_ref(),
        batch: Some(&scope),
        fail_closed_budget_enforcement: false,
    };

    let answer = reads.read_spend_counter_cache_value(KEY).await;

    assert_eq!(answer, (Some(decimal("5")), true));
}

#[tokio::test]
async fn the_dual_cache_stores_a_ttled_memory_value() {
    let memory = Arc::new(
        LocalMemoryOps::new(1_024, Arc::new(|| 0.0_f64)).expect("the local memory tier builds"),
    );
    let settings = DualCacheSettings::from_environment(&BTreeMap::new());
    let cache = DualCache::new(Arc::clone(&memory), None, settings);

    cache
        .memory_set_ttl(KEY, decimal("2.5"), Some(5.0_f64))
        .expect("the ttl write");
    cache
        .memory_set_ttl(FRESH_KEY, decimal("1"), None)
        .expect("the plain write");

    assert_eq!(
        cache.memory_get(KEY).await.expect("the memory read"),
        Some(ReseedCacheValue::Number(decimal("2.5")))
    );
    let entries = memory.ttl_entries().expect("the ttl entries");
    let with_ttl = entries
        .iter()
        .find(|(name, _expiry)| name.as_str() == KEY)
        .map(|(_name, expiry)| *expiry);
    assert_eq!(with_ttl.map(f64::to_bits), Some(5.0_f64.to_bits()));
    let plain = entries
        .iter()
        .find(|(name, _expiry)| name.as_str() == FRESH_KEY)
        .map(|(_name, expiry)| *expiry);
    assert!(plain.is_some_and(|expiry| expiry > 5.0_f64));
}

#[tokio::test]
async fn the_dual_cache_raise_answers_nothing_without_redis() {
    let memory = Arc::new(
        LocalMemoryOps::new(1_024, Arc::new(|| 0.0_f64)).expect("the local memory tier builds"),
    );
    let settings = DualCacheSettings::from_environment(&BTreeMap::new());
    let cache = DualCache::new(memory, None, settings);

    let repaired = cache
        .redis_set_max(KEY, decimal("5"))
        .await
        .expect("the raise");

    assert_eq!(repaired, None);
}
