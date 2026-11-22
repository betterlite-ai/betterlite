use rust_decimal::Decimal;

use super::support::{
    Captured, Fixture, KEY, RowAnswer, aggregate, aware, decimal, key_window, marker_key,
    window_row, written,
};
use crate::budget_counter::CounterEntityType;
use crate::spend_reads::request::SpendWindow;
use crate::spend_reseed::values::ReseedCacheValue;

const WINDOW_KEY: &str = "spend:key:token:window:1h";
const END_USER_KEY: &str = "spend:end_user:alice";

#[tokio::test]
async fn a_cached_marker_answers_without_a_database_read() {
    let fixture = Fixture::new(false);
    fixture
        .cache
        .seed_memory(&marker_key(KEY), ReseedCacheValue::Number(decimal("4")));
    fixture.rows.set_entity(RowAnswer::Spent(decimal("9")));

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(KEY, None)
        .await;

    assert_eq!(floor, Some(decimal("4")));
    assert!(fixture.rows.entity_reads().is_empty());
}

#[tokio::test]
async fn a_database_floor_is_cached_under_the_marker_key_with_the_five_second_ttl() {
    let fixture = Fixture::new(false);
    fixture.rows.set_entity(RowAnswer::Spent(decimal("6")));

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(KEY, None)
        .await;

    assert_eq!(floor, Some(decimal("6")));
    assert_eq!(fixture.rows.entity_reads(), ["key:token".to_owned()]);
    let writes = fixture.cache.memory_writes();
    let (value, ttl) = written(&writes, &marker_key(KEY)).expect("the floor is cached");
    assert_eq!(value, decimal("6"));
    assert_eq!(ttl.map(f64::to_bits), Some(5.0_f64.to_bits()));
}

#[tokio::test]
async fn a_null_row_is_a_zero_floor() {
    let fixture = Fixture::new(false);
    fixture.rows.set_entity(RowAnswer::Null);

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(KEY, None)
        .await;

    assert_eq!(floor, Some(Decimal::ZERO));
    let writes = fixture.cache.memory_writes();
    let (value, _ttl) = written(&writes, &marker_key(KEY)).expect("the floor is cached");
    assert_eq!(value, Decimal::ZERO);
}

#[tokio::test]
async fn a_missing_row_without_a_window_has_no_floor() {
    let fixture = Fixture::new(false);

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(KEY, None)
        .await;

    assert_eq!(floor, None);
    assert!(fixture.cache.memory_writes().is_empty());
}

#[tokio::test]
async fn an_entity_floor_answers_before_the_window_floor() {
    let fixture = Fixture::new(false);
    fixture.rows.set_entity(RowAnswer::Spent(decimal("6")));
    fixture
        .rows
        .set_window(window_row("2026-11-18T12:00:00Z", "8"));
    let entity = CounterEntityType::Key;
    let window = key_window(&entity, aware("2026-11-18T12:00:00Z"));

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(KEY, Some(&window))
        .await;

    assert_eq!(floor, Some(decimal("6")));
    assert!(fixture.rows.window_reads().is_empty());
}

#[tokio::test]
async fn a_window_row_answers_when_the_entity_row_is_missing() {
    let fixture = Fixture::new(false);
    fixture
        .rows
        .set_window(window_row("2026-11-18T12:00:00Z", "8"));
    let entity = CounterEntityType::Key;
    let window = key_window(&entity, aware("2026-11-18T12:00:00Z"));

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(WINDOW_KEY, Some(&window))
        .await;

    assert_eq!(floor, Some(decimal("8")));
    assert_eq!(fixture.rows.window_reads(), ["Key:token:1h".to_owned()]);
    let writes = fixture.cache.memory_writes();
    let (value, _ttl) = written(&writes, &marker_key(WINDOW_KEY)).expect("the floor is cached");
    assert_eq!(value, decimal("8"));
}

#[tokio::test]
async fn the_window_spend_logs_answer_when_no_window_row_exists() {
    let fixture = Fixture::new(false);
    fixture.rows.set_logs(vec![aggregate("7")]);
    let entity = CounterEntityType::Key;
    let window = key_window(&entity, aware("2026-11-18T12:00:00Z"));

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(WINDOW_KEY, Some(&window))
        .await;

    assert_eq!(floor, Some(decimal("7")));
    assert_eq!(fixture.rows.log_reads(), 1);
}

#[tokio::test]
async fn a_window_row_from_before_the_window_start_falls_back_to_the_logs() {
    let fixture = Fixture::new(false);
    fixture
        .rows
        .set_window(window_row("2026-11-18T11:00:00Z", "8"));
    let entity = CounterEntityType::Key;
    let window = key_window(&entity, aware("2026-11-18T12:00:00Z"));

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(WINDOW_KEY, Some(&window))
        .await;

    assert_eq!(floor, Some(Decimal::ZERO));
    assert_eq!(fixture.rows.log_reads(), 1);
}

#[tokio::test]
async fn an_end_user_floor_uses_the_end_user_row() {
    let fixture = Fixture::new(false);
    fixture.rows.set_entity(RowAnswer::Spent(decimal("3")));

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(END_USER_KEY, None)
        .await;

    assert_eq!(floor, Some(decimal("3")));
    assert_eq!(fixture.rows.entity_reads(), ["end_user:alice".to_owned()]);
}

#[tokio::test]
async fn an_end_user_counter_ignores_the_window_floor() {
    let fixture = Fixture::new(false);
    fixture
        .rows
        .set_window(window_row("2026-11-18T12:00:00Z", "8"));
    let entity = CounterEntityType::Key;
    let window = key_window(&entity, aware("2026-11-18T12:00:00Z"));

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(END_USER_KEY, Some(&window))
        .await;

    assert_eq!(floor, None);
    assert!(fixture.rows.window_reads().is_empty());
}

#[tokio::test]
async fn a_marker_written_during_the_database_read_is_kept() {
    let fixture = Fixture::new(false);
    fixture.rows.set_entity(RowAnswer::Spent(decimal("6")));
    fixture.race_marker(KEY, ReseedCacheValue::Number(decimal("12")));

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(KEY, None)
        .await;

    assert_eq!(floor, Some(decimal("12")));
    assert_eq!(fixture.rows.entity_reads(), ["key:token".to_owned()]);
    assert!(fixture.cache.memory_writes().is_empty());
}

#[tokio::test]
async fn a_marker_that_races_in_unreadable_is_replaced_by_the_database_floor() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(false);
    fixture.rows.set_entity(RowAnswer::Spent(decimal("6")));
    fixture.race_marker(KEY, ReseedCacheValue::Text("abc".to_owned()));

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(KEY, None)
        .await;

    assert_eq!(floor, Some(decimal("6")));
    let writes = fixture.cache.memory_writes();
    let (value, _ttl) =
        written(&writes, &marker_key(KEY)).expect("the database floor replaces the raced marker");
    assert_eq!(value, decimal("6"));
    assert!(logs.has("the spend counter value is not a number"));
}

#[tokio::test]
async fn an_unreadable_marker_reads_as_absent_and_is_replaced() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(false);
    fixture
        .cache
        .seed_memory(&marker_key(KEY), ReseedCacheValue::Text("abc".to_owned()));
    fixture.rows.set_entity(RowAnswer::Spent(decimal("6")));

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(KEY, None)
        .await;

    assert_eq!(floor, Some(decimal("6")));
    let writes = fixture.cache.memory_writes();
    let (value, _ttl) = written(&writes, &marker_key(KEY)).expect("the floor is replaced");
    assert_eq!(value, decimal("6"));
    assert!(logs.has("the spend counter value is not a number"));
}

#[tokio::test]
async fn a_failed_marker_write_keeps_the_database_floor() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(false);
    fixture.rows.set_entity(RowAnswer::Spent(decimal("6")));
    fixture.cache.fail_memory_writes();

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(KEY, None)
        .await;

    assert_eq!(floor, Some(decimal("6")));
    assert!(fixture.cache.memory_writes().is_empty());
    assert!(logs.has("the spend floor marker could not be cached"));
}

#[tokio::test]
async fn a_window_floor_read_error_logs_and_yields_no_floor() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(false);
    fixture
        .rows
        .set_window(window_row("2026-11-18T12:00:00Z", "\"abc\""));
    let entity = CounterEntityType::Key;
    let window = key_window(&entity, aware("2026-11-18T12:00:00Z"));

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(WINDOW_KEY, Some(&window))
        .await;

    assert_eq!(floor, None);
    assert!(logs.has("the window spend floor read failed"));
}

#[tokio::test]
async fn a_window_without_a_duration_reads_the_spend_logs() {
    let fixture = Fixture::new(false);
    fixture.rows.set_logs(vec![aggregate("7")]);
    let entity = CounterEntityType::Key;
    let window = SpendWindow {
        entity_type: &entity,
        entity_id: "token",
        window_duration: None,
        window_start: aware("2026-11-18T12:00:00Z"),
    };

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(WINDOW_KEY, Some(&window))
        .await;

    assert_eq!(floor, Some(decimal("7")));
    assert!(fixture.rows.window_reads().is_empty());
    assert_eq!(fixture.rows.log_reads(), 1);
}

#[tokio::test]
async fn a_failed_window_row_read_falls_back_to_the_spend_logs() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(false);
    fixture.rows.fail_window();
    fixture.rows.set_logs(vec![aggregate("7")]);
    let entity = CounterEntityType::Key;
    let window = key_window(&entity, aware("2026-11-18T12:00:00Z"));

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(WINDOW_KEY, Some(&window))
        .await;

    assert_eq!(floor, Some(decimal("7")));
    assert!(logs.has("SpendCounterReseed.window_from_table: failed"));
}

#[tokio::test]
async fn a_failed_spend_logs_read_leaves_no_floor() {
    let logs = Captured::default();
    let _guard = logs.install();
    let fixture = Fixture::new(false);
    fixture.rows.fail_logs();
    let entity = CounterEntityType::Key;
    let window = key_window(&entity, aware("2026-11-18T12:00:00Z"));

    let floor = fixture
        .reads(false)
        .authoritative_floor_spend(WINDOW_KEY, Some(&window))
        .await;

    assert_eq!(floor, None);
    assert!(logs.has("SpendCounterReseed.window_from_spend_logs: failed"));
}
