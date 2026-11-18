use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rust_decimal::Decimal;

use super::support::{
    FakeBatch, FakeCache, FakeError, FakeRows, Logs, LookupRecord, RedisAnswer, SetAnswer,
    answered, decimal, reseed, reseed_with, services, spend_row,
};
use crate::spend_counter_batch::batch::{CounterValue, SpendCounterBatchError};
use crate::spend_reseed::reseed::CoalescedError;
use crate::spend_reseed::rows::SpendRow;
use crate::spend_reseed::values::{ReseedCacheValue, SpendValueError};

#[tokio::test]
async fn all_primary_prefixes_and_empty_ids() {
    let rows = Arc::new(FakeRows::with_row(spend_row("2.5")));
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let cases = [
        ("spend:key:token", LookupRecord::Key("token".to_owned())),
        ("spend:key:", LookupRecord::Key(String::new())),
        (
            "spend:team_member:user:team",
            LookupRecord::TeamMember {
                user_id: "user".to_owned(),
                team_id: "team".to_owned(),
            },
        ),
        ("spend:team:t1", LookupRecord::Team("t1".to_owned())),
        ("spend:team:", LookupRecord::Team(String::new())),
        ("spend:user:u1", LookupRecord::User("u1".to_owned())),
        ("spend:user:", LookupRecord::User(String::new())),
        ("spend:org:o1", LookupRecord::Organization("o1".to_owned())),
        ("spend:org:", LookupRecord::Organization(String::new())),
        ("spend:project:p1", LookupRecord::Project("p1".to_owned())),
        ("spend:project:", LookupRecord::Project(String::new())),
    ];
    for (counter_key, expected) in cases {
        assert_eq!(counter.from_db(counter_key).await, Ok(Some(decimal("2.5"))));
        assert_eq!(rows.lookups().last(), Some(&expected));
    }
    assert_eq!(rows.lookups().len(), 11);
    let unmatched = [
        "other:key:t",
        "spend:tag:t",
        "spend:model_access_group:g",
        "spend:end_user:e",
    ];
    for counter_key in unmatched {
        assert_eq!(counter.from_db(counter_key).await, Ok(None));
    }
    assert_eq!(rows.lookups().len(), 11);
}

#[tokio::test]
async fn member_last_colon_split() {
    let rows = Arc::new(FakeRows::with_row(spend_row("1.25")));
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let outcome = counter
        .from_db("spend:team_member:user:with:colon:team")
        .await;
    assert_eq!(outcome, Ok(Some(decimal("1.25"))));
    assert_eq!(
        rows.lookups(),
        vec![LookupRecord::TeamMember {
            user_id: "user:with:colon".to_owned(),
            team_id: "team".to_owned(),
        }]
    );
    assert_eq!(counter.from_db("spend:team_member:user").await, Ok(None));
    assert_eq!(counter.from_db("spend:team_member:").await, Ok(None));
    assert_eq!(rows.lookups().len(), 1);
    let empty_ids = counter.from_db("spend:team_member::").await;
    assert_eq!(empty_ids, Ok(Some(decimal("1.25"))));
    assert_eq!(
        rows.lookups().last(),
        Some(&LookupRecord::TeamMember {
            user_id: String::new(),
            team_id: String::new(),
        })
    );
}

#[tokio::test]
async fn valid_window_only_suppresses_key_team() {
    let rows = Arc::new(FakeRows::with_row(spend_row("3")));
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let suppressed = [
        "spend:key:token:window:1h",
        "spend:team:t1:window:1d",
        "spend:key:token:window:1h:window:1d",
        // rpartition scans the whole key, so a suffix at the prefix boundary still counts
        "spend:key:window:1h",
        "spend:team:window:1d",
    ];
    for counter_key in suppressed {
        assert_eq!(counter.from_db(counter_key).await, Ok(None));
    }
    assert!(rows.lookups().is_empty());
    let looked_up = [
        "spend:key:token:window:soon",
        "spend:key:token:window:",
        "spend:key:token",
        "spend:team:t1:window:1g",
        "spend:user:u1:window:1h",
        "spend:team_member:u:t:window:1h",
        "spend:key:token:window:1h:window:soon",
    ];
    for counter_key in looked_up {
        assert_eq!(counter.from_db(counter_key).await, Ok(Some(decimal("3"))));
    }
    assert_eq!(rows.lookups().len(), 7);
}

#[tokio::test]
async fn primary_query_bound_gate_error_log_and_none() {
    let logs = Logs::capture();
    let blocked_rows = Arc::new(FakeRows::with_row(spend_row("9")));
    let blocked = reseed_with(Some(Arc::clone(&blocked_rows)), services(0), 0.05);
    assert_eq!(blocked.from_db("spend:key:token").await, Ok(None));
    assert!(blocked_rows.lookups().is_empty());
    assert!(blocked.stall_tracker.stalled_within(60.0));
    assert!(logs.contains("the bounded lookup failed"));

    let failing_rows = Arc::new(FakeRows::with_row(spend_row("9")));
    failing_rows.rows_fail.store(true, Ordering::Relaxed);
    let failing = reseed_with(Some(Arc::clone(&failing_rows)), services(25), 5.0);
    assert_eq!(failing.from_db("spend:key:token").await, Ok(None));
    assert_eq!(
        failing_rows.lookups(),
        vec![LookupRecord::Key("token".to_owned())]
    );
    assert!(logs.contains("the counter row read failed"));

    let empty_rows = Arc::new(FakeRows::empty());
    let empty = reseed_with(Some(Arc::clone(&empty_rows)), services(25), 5.0);
    assert_eq!(empty.from_db("spend:key:token").await, Ok(None));
    assert_eq!(empty_rows.lookups().len(), 1);
}

#[tokio::test]
async fn conversion_after_query_catch() {
    let cases = [
        (
            "\"abc\"",
            Err(SpendValueError::OutOfRange {
                text: "abc".to_owned(),
            }),
        ),
        ("[1]", Err(SpendValueError::NotNumeric { kind: "a list" })),
        (
            "{\"x\": 1}",
            Err(SpendValueError::NotNumeric { kind: "an object" }),
        ),
        ("0", Ok(Decimal::ZERO)),
        ("null", Ok(Decimal::ZERO)),
        ("false", Ok(Decimal::ZERO)),
        ("\"\"", Ok(Decimal::ZERO)),
        ("[]", Ok(Decimal::ZERO)),
        ("{}", Ok(Decimal::ZERO)),
        ("\" 2.5 \"", Ok(decimal("2.5"))),
        ("\"1e2\"", Ok(decimal("100"))),
    ];
    for (spend, expected) in cases {
        let counter = reseed(FakeRows::with_row(spend_row(spend)));
        assert_eq!(counter.from_db("spend:key:token").await, expected);
    }
    let absent_field = reseed(FakeRows::with_row(SpendRow { spend: None }));
    assert_eq!(
        absent_field.from_db("spend:key:token").await,
        Ok(Some(Decimal::ZERO))
    );
}

#[tokio::test]
async fn enduser_prefix_floor_read_and_no_tag_reseed() {
    let logs = Logs::capture();
    let rows = Arc::new(FakeRows::with_row(spend_row("4.25")));
    let floor = reseed_with(Some(Arc::clone(&rows)), services(0), 0.05);
    let floor_spend = floor.end_user_from_db("spend:end_user:u1").await;
    assert_eq!(floor_spend, Ok(Some(decimal("4.25"))));
    assert_eq!(rows.lookups(), vec![LookupRecord::EndUser("u1".to_owned())]);

    let ungated = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    assert_eq!(ungated.end_user_from_db("spend:key:t").await, Ok(None));
    let empty_floor = ungated.end_user_from_db("spend:end_user:").await;
    assert_eq!(empty_floor, Ok(Some(decimal("4.25"))));
    assert_eq!(
        rows.lookups().last(),
        Some(&LookupRecord::EndUser(String::new()))
    );
    assert_eq!(ungated.from_db("spend:tag:t").await, Ok(None));
    assert_eq!(ungated.from_db("spend:end_user:u1").await, Ok(None));
    assert_eq!(ungated.end_user_from_db("spend:tag:t").await, Ok(None));
    assert_eq!(rows.lookups().len(), 2);

    rows.rows_fail.store(true, Ordering::Relaxed);
    let failed = ungated.end_user_from_db("spend:end_user:u2").await;
    assert_eq!(failed, Ok(None));
    assert!(logs.contains("the counter row read failed"));
}

#[tokio::test]
async fn batch_hit_and_clean_miss_precedence() {
    let hit_rows = Arc::new(FakeRows::with_row(spend_row("2.5")));
    let hit = reseed_with(Some(Arc::clone(&hit_rows)), services(25), 5.0);
    let hit_cache = FakeCache::new(true)
        .with_redis(ReseedCacheValue::Number(decimal("8")))
        .with_memory("spend:key:t", ReseedCacheValue::Number(decimal("9")));
    let hit_batch = FakeBatch::new(Some(CounterValue::Amount(decimal("7.5"))));
    let batch_answer = hit
        .coalesced(&hit_cache, &hit_batch, "spend:key:t", false)
        .await;
    assert_eq!(answered(batch_answer), Some(decimal("7.5")));
    assert_eq!(hit_cache.redis_gets.load(Ordering::Relaxed), 0);
    assert_eq!(hit_cache.memory_gets.load(Ordering::Relaxed), 0);
    assert!(hit_rows.lookups().is_empty());

    let miss_rows = Arc::new(FakeRows::with_row(spend_row("2.5")));
    let miss = reseed_with(Some(Arc::clone(&miss_rows)), services(25), 5.0);
    let miss_cache = FakeCache::new(true)
        .with_redis(ReseedCacheValue::Number(decimal("8")))
        .with_memory("spend:key:t", ReseedCacheValue::Number(decimal("9")));
    let miss_batch = FakeBatch::new(Some(CounterValue::Missing));
    let miss_answer = miss
        .coalesced(&miss_cache, &miss_batch, "spend:key:t", false)
        .await;
    assert_eq!(answered(miss_answer), Some(decimal("2.5")));
    assert_eq!(miss_cache.redis_gets.load(Ordering::Relaxed), 0);
    assert_eq!(miss_cache.memory_gets.load(Ordering::Relaxed), 0);
    assert_eq!(miss_rows.lookups(), vec![LookupRecord::Key("t".to_owned())]);
    assert_eq!(
        miss_batch.recorded(),
        vec![("spend:key:t".to_owned(), decimal("2.5"))]
    );
}

#[tokio::test]
async fn no_batch_answer_reads_redis() {
    let rows = Arc::new(FakeRows::with_row(spend_row("2.5")));
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let cache = FakeCache::new(true).with_redis(ReseedCacheValue::Number(decimal("8")));
    let batch = FakeBatch::new(None);
    let outcome = counter
        .coalesced(&cache, &batch, "spend:key:t", false)
        .await;
    assert_eq!(answered(outcome), Some(decimal("8")));
    assert_eq!(cache.redis_gets.load(Ordering::Relaxed), 1);
    assert!(rows.lookups().is_empty());
}

#[tokio::test]
async fn batch_read_error_propagates() {
    let rows = Arc::new(FakeRows::with_row(spend_row("2.5")));
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let cache = FakeCache::new(true);
    let mut batch = FakeBatch::new(None);
    batch.fails = true;
    let outcome = counter
        .coalesced(&cache, &batch, "spend:key:t", false)
        .await;
    assert!(matches!(
        outcome,
        Err(CoalescedError::Batch(SpendCounterBatchError::Cancelled))
    ));
    assert!(rows.lookups().is_empty());
}

#[tokio::test]
async fn redis_hit_miss_error_local_fallback() {
    for (hit, expected) in [
        (ReseedCacheValue::Number(decimal("3")), decimal("3")),
        (ReseedCacheValue::Text(" 2.5 ".to_owned()), decimal("2.5")),
        (ReseedCacheValue::Boolean(true), decimal("1")),
    ] {
        let hit_rows = Arc::new(FakeRows::with_row(spend_row("9")));
        let hit_counter = reseed_with(Some(Arc::clone(&hit_rows)), services(25), 5.0);
        let hit_cache = FakeCache::new(true).with_redis(hit);
        let hit_batch = FakeBatch::new(None);
        let outcome = hit_counter
            .coalesced(&hit_cache, &hit_batch, "spend:key:t", false)
            .await;
        assert_eq!(answered(outcome), Some(expected));
        assert_eq!(hit_cache.memory_gets.load(Ordering::Relaxed), 0);
        assert!(hit_rows.lookups().is_empty());
    }

    let error_rows = Arc::new(FakeRows::with_row(spend_row("9")));
    let error_counter = reseed_with(Some(Arc::clone(&error_rows)), services(25), 5.0);
    let error_cache = FakeCache::new(true)
        .answer(RedisAnswer::Fail)
        .with_memory("spend:key:t", ReseedCacheValue::Number(decimal("4")));
    let error_batch = FakeBatch::new(None);
    let fallback = error_counter
        .coalesced(&error_cache, &error_batch, "spend:key:t", false)
        .await;
    assert_eq!(answered(fallback), Some(decimal("4")));
    assert!(error_rows.lookups().is_empty());

    let miss_rows = Arc::new(FakeRows::with_row(spend_row("2.5")));
    let miss_counter = reseed_with(Some(Arc::clone(&miss_rows)), services(25), 5.0);
    let miss_cache =
        FakeCache::new(true).with_memory("spend:key:t", ReseedCacheValue::Number(decimal("4")));
    let miss_batch = FakeBatch::new(None);
    let seeded = miss_counter
        .coalesced(&miss_cache, &miss_batch, "spend:key:t", false)
        .await;
    assert_eq!(answered(seeded), Some(decimal("2.5")));
    assert_eq!(miss_cache.memory_gets.load(Ordering::Relaxed), 0);
    assert_eq!(miss_rows.lookups().len(), 1);
}

#[tokio::test]
async fn unreadable_redis_falls_back_and_memory_conversion_propagates() {
    let unreadable_rows = Arc::new(FakeRows::with_row(spend_row("9")));
    let counter = reseed_with(Some(Arc::clone(&unreadable_rows)), services(25), 5.0);
    let cache = FakeCache::new(true)
        .with_redis(ReseedCacheValue::Unreadable)
        .with_memory("spend:key:t", ReseedCacheValue::Text("1.5".to_owned()));
    let batch = FakeBatch::new(None);
    let local = counter
        .coalesced(&cache, &batch, "spend:key:t", false)
        .await;
    assert_eq!(answered(local), Some(decimal("1.5")));
    assert!(unreadable_rows.lookups().is_empty());

    let broken_rows = Arc::new(FakeRows::with_row(spend_row("9")));
    let broken_counter = reseed_with(Some(Arc::clone(&broken_rows)), services(25), 5.0);
    let broken_cache = FakeCache::new(true)
        .answer(RedisAnswer::Fail)
        .with_memory("spend:key:t", ReseedCacheValue::Unreadable);
    let broken_batch = FakeBatch::new(None);
    let outcome = broken_counter
        .coalesced(&broken_cache, &broken_batch, "spend:key:t", false)
        .await;
    assert!(matches!(
        outcome,
        Err(CoalescedError::Spend(SpendValueError::NotNumeric {
            kind: "an unreadable cache value"
        }))
    ));
    assert!(broken_rows.lookups().is_empty());
}

#[tokio::test]
async fn local_increment_same_lock() {
    let rows = Arc::new(FakeRows::empty());
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let mut cache = FakeCache::new(false);
    cache.increment_delay = Duration::from_millis(20);
    let first = counter.increment_in_memory(&cache, "spend:key:t", decimal("1.5"));
    let second = counter.increment_in_memory(&cache, "spend:key:t", decimal("1.5"));
    let (left, right) = tokio::join!(first, second);
    assert!(matches!(left, Ok(Some(_))));
    assert!(matches!(right, Ok(Some(_))));
    assert_eq!(cache.increments().get("spend:key:t"), Some(&decimal("3")));

    cache.increment_fails.store(true, Ordering::Relaxed);
    let failed = counter
        .increment_in_memory(&cache, "spend:key:t", Decimal::ONE)
        .await;
    assert!(failed.is_err());
}

#[tokio::test]
async fn one_db_load_per_lock() {
    let mut delayed = FakeRows::with_row(spend_row("2.5"));
    delayed.delay = Duration::from_millis(20);
    let rows = Arc::new(delayed);
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let cache = FakeCache::new(true);
    let batch = FakeBatch::new(None);
    let first = counter.coalesced(&cache, &batch, "spend:key:t", false);
    let second = counter.coalesced(&cache, &batch, "spend:key:t", false);
    let (left, right) = tokio::join!(first, second);
    assert_eq!(answered(left), Some(decimal("2.5")));
    assert_eq!(answered(right), Some(decimal("2.5")));
    assert_eq!(rows.lookups().len(), 1);
}

#[tokio::test]
async fn zero_warmed() {
    let rows = Arc::new(FakeRows::with_row(spend_row("0")));
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let cache = FakeCache::new(true);
    let batch = FakeBatch::new(None);
    let outcome = counter
        .coalesced(&cache, &batch, "spend:key:t", false)
        .await;
    assert_eq!(answered(outcome), Some(Decimal::ZERO));
    assert_eq!(
        cache.redis_sets(),
        vec![("spend:key:t".to_owned(), Decimal::ZERO)]
    );
    assert_eq!(
        cache.memory_sets(),
        vec![("spend:key:t".to_owned(), Decimal::ZERO)]
    );
    assert_eq!(
        batch.recorded(),
        vec![("spend:key:t".to_owned(), Decimal::ZERO)]
    );

    let absent = reseed(FakeRows::with_row(SpendRow { spend: None }));
    let local_cache = FakeCache::new(false);
    let local_batch = FakeBatch::new(None);
    let zero = absent
        .coalesced(&local_cache, &local_batch, "spend:key:t", false)
        .await;
    assert_eq!(answered(zero), Some(Decimal::ZERO));
    assert_eq!(
        local_cache.memory_sets(),
        vec![("spend:key:t".to_owned(), Decimal::ZERO)]
    );
}

#[tokio::test]
async fn multipod_nx_winner_not_multiplied() {
    let winner_rows = Arc::new(FakeRows::with_row(spend_row("2.5")));
    let winner = reseed_with(Some(Arc::clone(&winner_rows)), services(25), 5.0);
    let winner_cache = FakeCache::new(true);
    let winner_batch = FakeBatch::new(None);
    let seeded = winner
        .coalesced(&winner_cache, &winner_batch, "spend:key:t", false)
        .await;
    assert_eq!(answered(seeded), Some(decimal("2.5")));
    assert_eq!(
        winner_cache.redis_sets(),
        vec![("spend:key:t".to_owned(), decimal("2.5"))]
    );
    assert!(winner_cache.increments().is_empty());

    let loser_rows = Arc::new(FakeRows::with_row(spend_row("2.5")));
    let loser = reseed_with(Some(Arc::clone(&loser_rows)), services(25), 5.0);
    let loser_cache = FakeCache::new(true)
        .with_redis(ReseedCacheValue::Number(decimal("5")))
        .answer(RedisAnswer::Miss)
        .with_set_answer(SetAnswer::Taken);
    let loser_batch = FakeBatch::new(None);
    let loser_answer = loser
        .coalesced(&loser_cache, &loser_batch, "spend:key:t", false)
        .await;
    assert_eq!(answered(loser_answer), Some(decimal("5")));
    assert_eq!(
        loser_cache.memory_sets(),
        vec![("spend:key:t".to_owned(), decimal("5"))]
    );
    assert_eq!(
        loser_batch.recorded(),
        vec![("spend:key:t".to_owned(), decimal("5"))]
    );
    assert!(loser_cache.increments().is_empty());
}

#[tokio::test]
async fn vanished_nx_winner_uses_db() {
    let rows = Arc::new(FakeRows::with_row(spend_row("2.5")));
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let cache = FakeCache::new(true).with_set_answer(SetAnswer::Taken);
    let batch = FakeBatch::new(None);
    let outcome = counter
        .coalesced(&cache, &batch, "spend:key:t", false)
        .await;
    assert_eq!(answered(outcome), Some(decimal("2.5")));
    assert_eq!(
        cache.memory_sets(),
        vec![("spend:key:t".to_owned(), decimal("2.5"))]
    );
    assert!(cache.increments().is_empty());
}

#[tokio::test]
async fn local_delta_max_preserved() {
    let delta_rows = Arc::new(FakeRows::with_row(spend_row("2.5")));
    let delta = reseed_with(Some(Arc::clone(&delta_rows)), services(25), 5.0);
    let delta_cache = FakeCache::new(false)
        .with_memory_answers([None, Some(ReseedCacheValue::Number(decimal("9")))]);
    let delta_batch = FakeBatch::new(None);
    let delta_answer = delta
        .coalesced(&delta_cache, &delta_batch, "spend:key:t", false)
        .await;
    assert_eq!(answered(delta_answer), Some(decimal("9")));
    assert_eq!(
        delta_cache.memory_sets(),
        vec![("spend:key:t".to_owned(), decimal("9"))]
    );

    let low_rows = Arc::new(FakeRows::with_row(spend_row("2.5")));
    let low = reseed_with(Some(Arc::clone(&low_rows)), services(25), 5.0);
    let low_cache = FakeCache::new(false)
        .with_memory_answers([None, Some(ReseedCacheValue::Number(decimal("1")))]);
    let low_batch = FakeBatch::new(None);
    let low_answer = low
        .coalesced(&low_cache, &low_batch, "spend:key:t", false)
        .await;
    assert_eq!(answered(low_answer), Some(decimal("2.5")));
    assert_eq!(
        low_cache.memory_sets(),
        vec![("spend:key:t".to_owned(), decimal("2.5"))]
    );

    let warm_rows = Arc::new(FakeRows::with_row(spend_row("2.5")));
    let warm = reseed_with(Some(Arc::clone(&warm_rows)), services(25), 5.0);
    let warm_cache =
        FakeCache::new(false).with_memory("spend:key:t", ReseedCacheValue::Number(decimal("4")));
    let warm_batch = FakeBatch::new(None);
    let warm_answer = warm
        .coalesced(&warm_cache, &warm_batch, "spend:key:t", false)
        .await;
    assert_eq!(answered(warm_answer), Some(decimal("4")));
    assert!(warm_rows.lookups().is_empty());
}

#[tokio::test]
async fn warm_failure_partial_effects_default_vs_required() {
    let logs = Logs::capture();
    let failed_rows = Arc::new(FakeRows::with_row(spend_row("2.5")));
    let failed = reseed_with(Some(Arc::clone(&failed_rows)), services(25), 5.0);
    let failed_cache = FakeCache::new(true).with_set_answer(SetAnswer::Fail);
    let failed_batch = FakeBatch::new(None);
    let defaulted = failed
        .coalesced(&failed_cache, &failed_batch, "spend:key:t", false)
        .await;
    assert_eq!(answered(defaulted), Some(decimal("2.5")));
    assert!(failed_cache.memory_sets().is_empty());
    assert!(failed_batch.recorded().is_empty());
    assert!(logs.contains("failed to warm the counter"));

    let strict_cache = FakeCache::new(true).with_set_answer(SetAnswer::Fail);
    let strict_batch = FakeBatch::new(None);
    let strict = failed
        .coalesced(&strict_cache, &strict_batch, "spend:key:t", true)
        .await;
    assert!(matches!(strict, Err(CoalescedError::Cache(FakeError))));

    let partial_rows = Arc::new(FakeRows::with_row(spend_row("2.5")));
    let partial = reseed_with(Some(Arc::clone(&partial_rows)), services(25), 5.0);
    let partial_cache = FakeCache::new(true)
        .with_redis(ReseedCacheValue::Number(decimal("5")))
        .answer(RedisAnswer::Miss)
        .with_set_answer(SetAnswer::Taken);
    partial_cache
        .memory_set_fails
        .store(true, Ordering::Relaxed);
    let partial_batch = FakeBatch::new(None);
    let partial_answer = partial
        .coalesced(&partial_cache, &partial_batch, "spend:key:t", false)
        .await;
    assert_eq!(answered(partial_answer), Some(decimal("5")));
    assert_eq!(
        partial_cache.redis_sets(),
        vec![("spend:key:t".to_owned(), decimal("2.5"))]
    );
    assert!(partial_batch.recorded().is_empty());
}
