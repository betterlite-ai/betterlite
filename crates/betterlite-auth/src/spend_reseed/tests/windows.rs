use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_db::budget_permission_rows::Timestamp;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{
    FakeBatch, FakeCache, FakeError, FakeRows, Logs, RedisAnswer, SetAnswer, answered, aware,
    decimal, mapping_sum, naive, reseed, reseed_with, services, spend_json, utc, window_spend_row,
};
use crate::budget_counter::CounterEntityType;
use crate::spend_counter_batch::batch::CounterValue;
use crate::spend_reseed::reseed::{CoalescedError, SpendCounterReseed};
use crate::spend_reseed::rows::{WindowEntity, WindowSpendRow};
use crate::spend_reseed::values::{ReseedCacheValue, SpendValueError};
use crate::spend_reseed::windows::{WindowQuery, WindowReseed};

const WINDOW_KEY: &str = "spend:key:k1:window:1h";

async fn table_answer(
    start: Timestamp,
    expected: Timestamp,
    spend: Option<&str>,
) -> Result<Option<Decimal>, SpendValueError> {
    let rows = FakeRows::default();
    let row = WindowSpendRow {
        window_start: start,
        spend: spend.map(spend_json),
    };
    *rows.window.lock().expect("the fake window row lock") = Some(row);
    let entity = CounterEntityType::Key;
    let query = WindowQuery {
        entity_type: &entity,
        entity_id: "k1",
        start,
    };
    reseed(rows).window_from_table(&query, "1h", expected).await
}

#[derive(Clone, Copy)]
enum WindowOp {
    Table,
    Logs,
    Db,
}

async fn window_spend(
    counter: &SpendCounterReseed<FakeRows>,
    start: Timestamp,
    op: WindowOp,
) -> Result<Option<Decimal>, SpendValueError> {
    let entity = CounterEntityType::Key;
    let query = WindowQuery {
        entity_type: &entity,
        entity_id: "k1",
        start,
    };
    match op {
        WindowOp::Table => counter.window_from_table(&query, "1h", start).await,
        WindowOp::Logs => counter.window_from_spend_logs(&query).await,
        WindowOp::Db => {
            counter
                .window_from_db(&entity, "k1", Some("1h"), start)
                .await
        }
    }
}

fn out_of_range() -> Result<Option<Decimal>, SpendValueError> {
    Err(SpendValueError::OutOfRange {
        text: "abc".to_owned(),
    })
}

#[rstest]
#[case::team_member(CounterEntityType::TeamMember)]
#[case::user(CounterEntityType::User)]
#[case::end_user(CounterEntityType::EndUser)]
#[case::tag(CounterEntityType::Tag)]
#[case::model_access_group(CounterEntityType::ModelAccessGroup)]
#[case::organization(CounterEntityType::Organization)]
#[case::project(CounterEntityType::Project)]
#[tokio::test]
async fn window_entity_mapping_and_compound_key(#[case] entity_type: CounterEntityType) {
    assert_eq!(WindowEntity::from_counter_type(&entity_type), None);
    assert_eq!(
        WindowEntity::from_counter_type(&CounterEntityType::Key),
        Some(WindowEntity::Key)
    );
    assert_eq!(
        WindowEntity::from_counter_type(&CounterEntityType::Team),
        Some(WindowEntity::Team)
    );
    assert_eq!(WindowEntity::Key.table_label(), "key");
    assert_eq!(WindowEntity::Team.table_label(), "team");

    let start = naive("2026-11-18T00:00:00");
    let rows = Arc::new(FakeRows::default());
    *rows.window.lock().expect("the fake window row lock") = Some(window_spend_row(start, "6.5"));
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    assert_eq!(
        window_spend(&counter, start, WindowOp::Table).await,
        Ok(Some(decimal("6.5")))
    );
    assert_eq!(
        rows.window_lookups(),
        vec![(WindowEntity::Key, "k1".to_owned(), "1h".to_owned())]
    );
    let missing = reseed_with(None, services(25), 5.0);
    assert_eq!(
        window_spend(&missing, start, WindowOp::Table).await,
        Ok(None)
    );
}

#[tokio::test]
async fn window_table_team_and_user_lookups() {
    let start = naive("2026-11-18T00:00:00");
    let rows = Arc::new(FakeRows::default());
    *rows.window.lock().expect("the fake window row lock") = Some(window_spend_row(start, "6.5"));
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let team_type = CounterEntityType::Team;
    let team = counter
        .window_from_table(
            &WindowQuery {
                entity_type: &team_type,
                entity_id: "t1",
                start,
            },
            "1d",
            start,
        )
        .await;
    assert_eq!(team, Ok(Some(decimal("6.5"))));
    assert_eq!(
        rows.window_lookups().last(),
        Some(&(WindowEntity::Team, "t1".to_owned(), "1d".to_owned()))
    );
    let user_type = CounterEntityType::User;
    let user = counter
        .window_from_table(
            &WindowQuery {
                entity_type: &user_type,
                entity_id: "u1",
                start,
            },
            "1d",
            start,
        )
        .await;
    // a user window counter has no maintained row, so it reads no table row at all
    assert_eq!(user, Ok(None));
    assert_eq!(rows.window_lookups().len(), 1);
}

#[rstest]
#[case::old_aware(aware("2026-11-17T23:59:59Z"), Ok(None))]
#[case::equal_aware(aware("2026-11-18T00:00:00Z"), Ok(Some(decimal("6.5"))))]
#[case::future_aware(aware("2026-11-19T00:00:00Z"), Ok(Some(decimal("6.5"))))]
#[case::equal_naive(naive("2026-11-18T00:00:00"), Ok(Some(decimal("6.5"))))]
#[case::old_naive(naive("2026-11-17T23:59:59"), Ok(None))]
#[case::future_naive(naive("2026-11-18T00:00:01"), Ok(Some(decimal("6.5"))))]
#[case::earlier_offset(aware("2026-11-18T01:00:00+02:00"), Ok(None))]
#[case::offset_in_utc(aware("2026-11-18T02:00:00+02:00"), Ok(Some(decimal("6.5"))))]
#[tokio::test]
async fn window_old_equal_future_naive_and_aware(
    #[case] start: Timestamp,
    #[case] expected_answer: Result<Option<Decimal>, SpendValueError>,
) {
    let expected = aware("2026-11-18T00:00:00Z");
    assert_eq!(
        table_answer(start, expected, Some("6.5")).await,
        expected_answer
    );
}

#[tokio::test]
async fn window_naive_expected_start_and_falsey_stored_spend() {
    let expected = aware("2026-11-18T00:00:00Z");
    // a naive expected start compares against an aware row start in UTC
    assert_eq!(
        table_answer(
            aware("2026-11-18T00:00:00Z"),
            naive("2026-11-18T00:00:00"),
            Some("6.5")
        )
        .await,
        Ok(Some(decimal("6.5")))
    );
    // a falsey stored spend answers zero
    assert_eq!(
        table_answer(expected, expected, Some("null")).await,
        Ok(Some(Decimal::ZERO))
    );
    assert_eq!(
        table_answer(expected, expected, None).await,
        Ok(Some(Decimal::ZERO))
    );
}

#[tokio::test]
async fn absent_duration_skips_maintained_row() {
    let start = naive("2026-11-18T00:00:00");
    let rows = Arc::new(FakeRows::default());
    *rows.logs.lock().expect("the fake logs lock") = vec![mapping_sum("8")];
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let no_duration = counter
        .window_from_db(&CounterEntityType::Key, "k1", None, start)
        .await;
    assert_eq!(no_duration, Ok(Some(decimal("8"))));
    assert!(rows.window_lookups().is_empty());
    assert_eq!(rows.log_lookups().len(), 1);

    *rows.window.lock().expect("the fake window row lock") = Some(window_spend_row(start, "3"));
    assert_eq!(
        window_spend(&counter, start, WindowOp::Db).await,
        Ok(Some(decimal("3")))
    );
    assert_eq!(rows.window_lookups().len(), 1);
    assert_eq!(rows.log_lookups().len(), 1);

    let stale = naive("2026-11-17T00:00:00");
    *rows.window.lock().expect("the fake window row lock") = Some(window_spend_row(stale, "3"));
    assert_eq!(
        window_spend(&counter, start, WindowOp::Db).await,
        Ok(Some(decimal("8")))
    );
    assert_eq!(rows.window_lookups().len(), 2);
    assert_eq!(rows.log_lookups().len(), 2);

    let unsupported = counter
        .window_from_db(&CounterEntityType::User, "u1", Some("1h"), start)
        .await;
    assert_eq!(unsupported, Ok(None));
    assert_eq!(rows.window_lookups().len(), 2);
}

#[tokio::test]
async fn aggregate_entity_start_filter_empty_and_first_row() {
    let start = aware("2026-11-18T00:00:00Z");
    let rows = Arc::new(FakeRows::default());
    *rows.logs.lock().expect("the fake logs lock") = vec![mapping_sum("7.5"), mapping_sum("100")];
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    assert_eq!(
        window_spend(&counter, start, WindowOp::Logs).await,
        Ok(Some(decimal("7.5")))
    );
    assert_eq!(
        rows.log_lookups(),
        vec![(
            WindowEntity::Key,
            "k1".to_owned(),
            utc("2026-11-18T00:00:00Z")
        )]
    );
    *rows.logs.lock().expect("the fake logs lock") = Vec::new();
    assert_eq!(
        window_spend(&counter, start, WindowOp::Logs).await,
        Ok(Some(Decimal::ZERO))
    );
    let tag_type = CounterEntityType::Tag;
    let unsupported = counter
        .window_from_spend_logs(&WindowQuery {
            entity_type: &tag_type,
            entity_id: "tag",
            start,
        })
        .await;
    assert_eq!(unsupported, Ok(None));
    let rowless = reseed_with(None, services(25), 5.0);
    assert_eq!(
        window_spend(&rowless, start, WindowOp::Logs).await,
        Ok(None)
    );
}

#[tokio::test]
async fn aggregate_team_first_row_and_failed_logs() {
    let start = aware("2026-11-18T00:00:00Z");
    let rows = Arc::new(FakeRows::default());
    *rows.logs.lock().expect("the fake logs lock") = vec![mapping_sum("7.5"), mapping_sum("100")];
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let team_type = CounterEntityType::Team;
    let team_first = counter
        .window_from_spend_logs(&WindowQuery {
            entity_type: &team_type,
            entity_id: "t1",
            start,
        })
        .await;
    assert_eq!(team_first, Ok(Some(decimal("7.5"))));
    assert_eq!(
        rows.log_lookups().last(),
        Some(&(
            WindowEntity::Team,
            "t1".to_owned(),
            utc("2026-11-18T00:00:00Z")
        ))
    );

    rows.logs_fail.store(true, Ordering::Relaxed);
    assert_eq!(
        window_spend(&counter, start, WindowOp::Logs).await,
        Ok(None)
    );
}

#[tokio::test]
async fn query_failure_falls_back_but_conversion_propagates() {
    let logs = Logs::capture();
    let start = aware("2026-11-18T00:00:00Z");
    let rows = Arc::new(FakeRows::default());
    *rows.window.lock().expect("the fake window row lock") =
        Some(window_spend_row(start, "\"abc\""));
    *rows.logs.lock().expect("the fake logs lock") = vec![mapping_sum("\"abc\"")];
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    assert_eq!(
        window_spend(&counter, start, WindowOp::Table).await,
        out_of_range()
    );
    assert_eq!(
        window_spend(&counter, start, WindowOp::Logs).await,
        out_of_range()
    );
    assert_eq!(
        window_spend(&counter, start, WindowOp::Db).await,
        out_of_range()
    );

    rows.window_fails.store(true, Ordering::Relaxed);
    rows.logs_fail.store(true, Ordering::Relaxed);
    assert_eq!(
        window_spend(&counter, start, WindowOp::Table).await,
        Ok(None)
    );
    assert_eq!(
        window_spend(&counter, start, WindowOp::Logs).await,
        Ok(None)
    );
    assert!(logs.contains("window_from_table: failed"));
    assert!(logs.contains("window_from_spend_logs: failed"));
}

#[tokio::test]
async fn stale_window_row_reads_the_logs_aggregate() {
    let start = aware("2026-11-18T00:00:00Z");
    let rows = Arc::new(FakeRows::default());
    *rows.window.lock().expect("the fake window row lock") =
        Some(window_spend_row(naive("2026-11-17T00:00:00"), "20"));
    *rows.logs.lock().expect("the fake logs lock") = vec![mapping_sum("8")];
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    assert_eq!(
        window_spend(&counter, start, WindowOp::Db).await,
        Ok(Some(decimal("8")))
    );
}

const EXPECTED_START: &str = "2026-11-18T00:00:00Z";

fn window_rows_with(spend: &str) -> Arc<FakeRows> {
    let rows = Arc::new(FakeRows::default());
    *rows.window.lock().expect("the fake window row lock") =
        Some(window_spend_row(naive("2026-11-18T00:00:00"), spend));
    rows
}

async fn coalesced_window(
    counter: &SpendCounterReseed<FakeRows>,
    cache: &FakeCache,
    batch: &FakeBatch,
    duration: Option<&str>,
) -> Result<Option<Decimal>, CoalescedError<FakeError>> {
    let entity = CounterEntityType::Key;
    let request = WindowReseed {
        counter_key: WINDOW_KEY,
        entity_type: &entity,
        entity_id: "k1",
        window_duration: duration,
        window_start: aware(EXPECTED_START),
    };
    counter.coalesced_window(cache, batch, &request).await
}

#[tokio::test]
async fn window_batch_hit_and_clean_miss_precedence() {
    let hit_rows = window_rows_with("6.5");
    let hit = reseed_with(Some(Arc::clone(&hit_rows)), services(25), 5.0);
    let hit_cache = FakeCache::new(true)
        .with_redis(ReseedCacheValue::Number(decimal("8")))
        .with_memory(WINDOW_KEY, ReseedCacheValue::Number(decimal("9")));
    let hit_batch = FakeBatch::new(Some(CounterValue::Amount(decimal("7.5"))));
    let hit_answer = coalesced_window(&hit, &hit_cache, &hit_batch, Some("1h")).await;
    assert_eq!(answered(hit_answer), Some(decimal("7.5")));
    assert_eq!(hit_cache.redis_gets.load(Ordering::Relaxed), 0);
    assert_eq!(hit_cache.memory_gets.load(Ordering::Relaxed), 0);
    assert!(hit_rows.window_lookups().is_empty());

    // a batched miss is an authoritative Redis miss, so the stale per-pod memory never answers
    let miss_rows = window_rows_with("2.5");
    let miss = reseed_with(Some(Arc::clone(&miss_rows)), services(25), 5.0);
    let miss_cache = FakeCache::new(true)
        .with_redis(ReseedCacheValue::Number(decimal("8")))
        .with_memory(WINDOW_KEY, ReseedCacheValue::Number(decimal("9")));
    let miss_batch = FakeBatch::new(Some(CounterValue::Missing));
    let miss_answer = coalesced_window(&miss, &miss_cache, &miss_batch, Some("1h")).await;
    assert_eq!(answered(miss_answer), Some(decimal("2.5")));
    assert_eq!(miss_cache.redis_gets.load(Ordering::Relaxed), 0);
    assert_eq!(miss_cache.memory_gets.load(Ordering::Relaxed), 0);
    assert_eq!(miss_rows.window_lookups().len(), 1);
    assert_eq!(
        miss_batch.recorded(),
        vec![(WINDOW_KEY.to_owned(), decimal("2.5"))]
    );
}

#[tokio::test]
async fn window_redis_hit_clean_miss_and_failure_precedence() {
    let hit_rows = window_rows_with("2.5");
    let hit = reseed_with(Some(Arc::clone(&hit_rows)), services(25), 5.0);
    let hit_cache = FakeCache::new(true).with_redis(ReseedCacheValue::Number(decimal("8")));
    let hit_batch = FakeBatch::new(None);
    let hit_answer = coalesced_window(&hit, &hit_cache, &hit_batch, Some("1h")).await;
    assert_eq!(answered(hit_answer), Some(decimal("8")));
    assert_eq!(hit_cache.memory_gets.load(Ordering::Relaxed), 0);
    assert!(hit_rows.window_lookups().is_empty());

    // a successful Redis miss suppresses local memory and reads the authoritative window
    let miss_rows = window_rows_with("2.5");
    let miss = reseed_with(Some(Arc::clone(&miss_rows)), services(25), 5.0);
    let miss_cache =
        FakeCache::new(true).with_memory(WINDOW_KEY, ReseedCacheValue::Number(decimal("4")));
    let miss_batch = FakeBatch::new(None);
    let miss_answer = coalesced_window(&miss, &miss_cache, &miss_batch, Some("1h")).await;
    assert_eq!(answered(miss_answer), Some(decimal("2.5")));
    assert_eq!(miss_cache.memory_gets.load(Ordering::Relaxed), 0);
    assert_eq!(miss_rows.window_lookups().len(), 1);

    // only a failed Redis read leaves the per-pod value as the answer
    let error_rows = window_rows_with("2.5");
    let error = reseed_with(Some(Arc::clone(&error_rows)), services(25), 5.0);
    let error_cache = FakeCache::new(true)
        .answer(RedisAnswer::Fail)
        .with_memory(WINDOW_KEY, ReseedCacheValue::Number(decimal("4")));
    let error_batch = FakeBatch::new(None);
    let error_answer = coalesced_window(&error, &error_cache, &error_batch, Some("1h")).await;
    assert_eq!(answered(error_answer), Some(decimal("4")));
    assert!(error_rows.window_lookups().is_empty());
}

#[tokio::test]
async fn window_lost_nx_increments_when_the_winner_vanished() {
    let rows = window_rows_with("2.5");
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let cache = FakeCache::new(true).with_set_answer(SetAnswer::Taken);
    let batch = FakeBatch::new(None);
    let answer = coalesced_window(&counter, &cache, &batch, Some("1h")).await;
    assert_eq!(answered(answer), Some(decimal("2.5")));
    assert_eq!(
        cache.redis_increments(),
        vec![(WINDOW_KEY.to_owned(), decimal("2.5"))]
    );
    assert_eq!(
        cache.memory_sets(),
        vec![(WINDOW_KEY.to_owned(), decimal("2.5"))]
    );
    assert_eq!(
        batch.recorded(),
        vec![(WINDOW_KEY.to_owned(), decimal("2.5"))]
    );
}

#[tokio::test]
async fn window_lost_nx_reads_the_winner_without_incrementing() {
    let rows = window_rows_with("2.5");
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let cache = FakeCache::new(true)
        .with_redis(ReseedCacheValue::Number(decimal("5")))
        // the recheck misses first; the winner is only visible to the losing seeder's reread
        .answer(RedisAnswer::Miss)
        .with_set_answer(SetAnswer::Taken);
    let batch = FakeBatch::new(None);
    let answer = coalesced_window(&counter, &cache, &batch, Some("1h")).await;
    assert_eq!(answered(answer), Some(decimal("5")));
    assert!(cache.redis_increments().is_empty());
    assert_eq!(
        cache.memory_sets(),
        vec![(WINDOW_KEY.to_owned(), decimal("5"))]
    );
    assert_eq!(
        batch.recorded(),
        vec![(WINDOW_KEY.to_owned(), decimal("5"))]
    );
}

#[tokio::test]
async fn window_memory_keeps_the_higher_local_spend_and_records_nothing() {
    let high_rows = window_rows_with("2.5");
    let high = reseed_with(Some(Arc::clone(&high_rows)), services(25), 5.0);
    // the recheck misses, so only the warm step's read sees the local value
    let high_cache = FakeCache::new(false)
        .with_memory_answers([None, Some(ReseedCacheValue::Number(decimal("9")))]);
    let high_batch = FakeBatch::new(None);
    let high_answer = coalesced_window(&high, &high_cache, &high_batch, Some("1h")).await;
    assert_eq!(answered(high_answer), Some(decimal("9")));
    assert_eq!(
        high_cache.memory_sets(),
        vec![(WINDOW_KEY.to_owned(), decimal("9"))]
    );
    assert!(high_batch.recorded().is_empty());

    let low_rows = window_rows_with("2.5");
    let low = reseed_with(Some(Arc::clone(&low_rows)), services(25), 5.0);
    let low_cache = FakeCache::new(false)
        .with_memory_answers([None, Some(ReseedCacheValue::Number(decimal("1")))]);
    let low_batch = FakeBatch::new(None);
    let low_answer = coalesced_window(&low, &low_cache, &low_batch, Some("1h")).await;
    assert_eq!(answered(low_answer), Some(decimal("2.5")));
    assert_eq!(
        low_cache.memory_sets(),
        vec![(WINDOW_KEY.to_owned(), decimal("2.5"))]
    );
}

#[tokio::test]
async fn window_warm_failures_always_propagate() {
    let logs = Logs::capture();
    let rows = window_rows_with("2.5");
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let batch = FakeBatch::new(None);

    let seed_failed = FakeCache::new(true).with_set_answer(SetAnswer::Fail);
    let seeded = coalesced_window(&counter, &seed_failed, &batch, Some("1h")).await;
    assert!(matches!(seeded, Err(CoalescedError::Cache(FakeError))));
    assert!(logs.contains("coalesced_window: failed to warm the counter"));

    let reread_failed = FakeCache::new(true)
        // the recheck fails first, then the losing seeder's reread fails
        .answer(RedisAnswer::Fail)
        .answer(RedisAnswer::Fail)
        .with_set_answer(SetAnswer::Taken);
    let reread = coalesced_window(&counter, &reread_failed, &batch, Some("1h")).await;
    assert!(matches!(reread, Err(CoalescedError::Cache(FakeError))));
    assert!(reread_failed.memory_sets().is_empty());

    let local_write_failed = FakeCache::new(true);
    local_write_failed
        .memory_set_fails
        .store(true, Ordering::Relaxed);
    let local = coalesced_window(&counter, &local_write_failed, &batch, Some("1h")).await;
    assert!(matches!(local, Err(CoalescedError::Cache(FakeError))));
    assert!(batch.recorded().is_empty());

    let increment_failed = FakeCache::new(true)
        .answer(RedisAnswer::Miss)
        .with_set_answer(SetAnswer::Taken);
    increment_failed
        .redis_increment_fails
        .store(true, Ordering::Relaxed);
    let incremented = coalesced_window(&counter, &increment_failed, &batch, Some("1h")).await;
    assert!(matches!(incremented, Err(CoalescedError::Cache(FakeError))));
}

#[tokio::test]
async fn window_unavailable_sources_answer_none_without_warming() {
    let rowless = reseed_with(None, services(25), 5.0);
    let cache = FakeCache::new(true);
    let batch = FakeBatch::new(None);
    assert_eq!(
        answered(coalesced_window(&rowless, &cache, &batch, Some("1h")).await),
        None
    );
    assert!(cache.redis_sets().is_empty());
    assert!(batch.recorded().is_empty());

    let rows = Arc::new(FakeRows::default());
    let unsupported = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let entity = CounterEntityType::User;
    let request = WindowReseed {
        counter_key: WINDOW_KEY,
        entity_type: &entity,
        entity_id: "u1",
        window_duration: Some("1h"),
        window_start: aware(EXPECTED_START),
    };
    let answer = unsupported.coalesced_window(&cache, &batch, &request).await;
    assert_eq!(answered(answer), None);
    assert!(rows.window_lookups().is_empty());
    assert!(cache.redis_sets().is_empty());
}

#[tokio::test]
async fn window_future_reset_row_is_trusted_and_null_duration_warms_zero() {
    let future_rows = Arc::new(FakeRows::default());
    *future_rows.window.lock().expect("the fake window row lock") =
        Some(window_spend_row(naive("2026-11-19T00:00:00"), "6.5"));
    let future = reseed_with(Some(Arc::clone(&future_rows)), services(25), 5.0);
    let future_cache = FakeCache::new(true);
    let future_batch = FakeBatch::new(None);
    let future_answer = coalesced_window(&future, &future_cache, &future_batch, Some("1h")).await;
    assert_eq!(answered(future_answer), Some(decimal("6.5")));
    assert!(future_rows.log_lookups().is_empty());

    let null_rows = window_rows_with("2.5");
    let null = reseed_with(Some(Arc::clone(&null_rows)), services(25), 5.0);
    let null_cache = FakeCache::new(true);
    let null_batch = FakeBatch::new(None);
    let null_answer = coalesced_window(&null, &null_cache, &null_batch, None).await;
    assert_eq!(answered(null_answer), Some(Decimal::ZERO));
    assert!(null_rows.window_lookups().is_empty());
    assert_eq!(null_rows.log_lookups().len(), 1);
    assert_eq!(
        null_cache.redis_sets(),
        vec![(WINDOW_KEY.to_owned(), Decimal::ZERO)]
    );
    assert_eq!(
        null_batch.recorded(),
        vec![(WINDOW_KEY.to_owned(), Decimal::ZERO)]
    );
}

#[tokio::test]
async fn one_window_db_load_per_lock() {
    let mut delayed = FakeRows::default();
    delayed.window_delay = Duration::from_millis(20);
    *delayed.window.lock().expect("the fake window row lock") =
        Some(window_spend_row(naive("2026-11-18T00:00:00"), "2.5"));
    let rows = Arc::new(delayed);
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let cache = FakeCache::new(true);
    let batch = FakeBatch::new(None);
    let first = coalesced_window(&counter, &cache, &batch, Some("1h"));
    let second = coalesced_window(&counter, &cache, &batch, Some("1h"));
    let (left, right) = tokio::join!(first, second);
    assert_eq!(answered(left), Some(decimal("2.5")));
    assert_eq!(answered(right), Some(decimal("2.5")));
    assert_eq!(rows.window_lookups().len(), 1);
    assert_eq!(cache.redis_sets().len(), 1);
}
