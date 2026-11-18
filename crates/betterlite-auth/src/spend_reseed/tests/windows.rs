use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_db::budget_permission_rows::Timestamp;
use rust_decimal::Decimal;

use super::support::{
    FakeRows, Logs, aware, decimal, mapping_sum, naive, reseed, reseed_with, services, spend_json,
    utc, window_spend_row,
};
use crate::budget_counter::CounterEntityType;
use crate::spend_reseed::reseed::SpendCounterReseed;
use crate::spend_reseed::rows::{WindowEntity, WindowSpendRow};
use crate::spend_reseed::values::SpendValueError;

async fn table_answer(
    start: Timestamp,
    expected: Timestamp,
) -> Result<Option<Decimal>, SpendValueError> {
    let rows = FakeRows::empty();
    *rows.window.lock().expect("the fake window row lock") = Some(window_spend_row(start, "6.5"));
    reseed(rows)
        .window_from_table(&CounterEntityType::Key, "k1", "1h", expected)
        .await
}

async fn table_answer_with_spend(
    expected: Timestamp,
    spend: Option<&str>,
) -> Result<Option<Decimal>, SpendValueError> {
    let rows = FakeRows::empty();
    let row = WindowSpendRow {
        window_start: expected,
        spend: spend.map(spend_json),
    };
    *rows.window.lock().expect("the fake window row lock") = Some(row);
    reseed(rows)
        .window_from_table(&CounterEntityType::Key, "k1", "1h", expected)
        .await
}

async fn table_spend(
    counter: &SpendCounterReseed<FakeRows>,
    start: Timestamp,
) -> Result<Option<Decimal>, SpendValueError> {
    counter
        .window_from_table(&CounterEntityType::Key, "k1", "1h", start)
        .await
}

async fn logs_spend(
    counter: &SpendCounterReseed<FakeRows>,
    start: Timestamp,
) -> Result<Option<Decimal>, SpendValueError> {
    counter
        .window_from_spend_logs(&CounterEntityType::Key, "k1", start)
        .await
}

async fn db_spend(
    counter: &SpendCounterReseed<FakeRows>,
    start: Timestamp,
) -> Result<Option<Decimal>, SpendValueError> {
    counter
        .window_from_db(&CounterEntityType::Key, "k1", Some("1h"), start)
        .await
}

fn out_of_range() -> Result<Option<Decimal>, SpendValueError> {
    Err(SpendValueError::OutOfRange {
        text: "abc".to_owned(),
    })
}

#[tokio::test]
async fn window_entity_mapping_and_compound_key() {
    assert_eq!(
        WindowEntity::from_counter_type(&CounterEntityType::Key),
        Some(WindowEntity::Key)
    );
    assert_eq!(
        WindowEntity::from_counter_type(&CounterEntityType::Team),
        Some(WindowEntity::Team)
    );
    let unsupported = [
        CounterEntityType::TeamMember,
        CounterEntityType::User,
        CounterEntityType::EndUser,
        CounterEntityType::Tag,
        CounterEntityType::ModelAccessGroup,
        CounterEntityType::Organization,
        CounterEntityType::Project,
    ];
    for entity_type in unsupported {
        assert_eq!(WindowEntity::from_counter_type(&entity_type), None);
    }
    assert_eq!(WindowEntity::Key.table_label(), "key");
    assert_eq!(WindowEntity::Team.table_label(), "team");

    let start = naive("2026-11-18T00:00:00");
    let rows = Arc::new(FakeRows::empty());
    *rows.window.lock().expect("the fake window row lock") = Some(window_spend_row(start, "6.5"));
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    assert_eq!(table_spend(&counter, start).await, Ok(Some(decimal("6.5"))));
    assert_eq!(
        rows.window_lookups(),
        vec![(WindowEntity::Key, "k1".to_owned(), "1h".to_owned())]
    );
    let team = counter
        .window_from_table(&CounterEntityType::Team, "t1", "1d", start)
        .await;
    assert_eq!(team, Ok(Some(decimal("6.5"))));
    assert_eq!(
        rows.window_lookups().last(),
        Some(&(WindowEntity::Team, "t1".to_owned(), "1d".to_owned()))
    );
    let user = counter
        .window_from_table(&CounterEntityType::User, "u1", "1d", start)
        .await;
    assert_eq!(user, Ok(None));
    assert_eq!(rows.window_lookups().len(), 2);
    let missing = reseed_with(None, services(25), 5.0);
    assert_eq!(table_spend(&missing, start).await, Ok(None));
}

#[tokio::test]
async fn window_old_equal_future_naive_and_aware() {
    let expected = aware("2026-11-18T00:00:00Z");
    let cases = [
        (aware("2026-11-17T23:59:59Z"), Ok(None)),
        (aware("2026-11-18T00:00:00Z"), Ok(Some(decimal("6.5")))),
        (aware("2026-11-19T00:00:00Z"), Ok(Some(decimal("6.5")))),
        (naive("2026-11-18T00:00:00"), Ok(Some(decimal("6.5")))),
        (naive("2026-11-17T23:59:59"), Ok(None)),
        (naive("2026-11-18T00:00:01"), Ok(Some(decimal("6.5")))),
        (aware("2026-11-18T01:00:00+02:00"), Ok(None)),
        (aware("2026-11-18T02:00:00+02:00"), Ok(Some(decimal("6.5")))),
    ];
    for (start, expected_answer) in cases {
        assert_eq!(table_answer(start, expected).await, expected_answer);
    }
    // a naive expected start compares against an aware row start in UTC
    assert_eq!(
        table_answer(aware("2026-11-18T00:00:00Z"), naive("2026-11-18T00:00:00")).await,
        Ok(Some(decimal("6.5")))
    );
    // a falsey stored spend answers zero
    assert_eq!(
        table_answer_with_spend(expected, Some("null")).await,
        Ok(Some(Decimal::ZERO))
    );
    assert_eq!(
        table_answer_with_spend(expected, None).await,
        Ok(Some(Decimal::ZERO))
    );
}

#[tokio::test]
async fn absent_duration_skips_maintained_row() {
    let start = naive("2026-11-18T00:00:00");
    let rows = Arc::new(FakeRows::empty());
    *rows.logs.lock().expect("the fake logs lock") = vec![mapping_sum("8")];
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    let no_duration = counter
        .window_from_db(&CounterEntityType::Key, "k1", None, start)
        .await;
    assert_eq!(no_duration, Ok(Some(decimal("8"))));
    assert!(rows.window_lookups().is_empty());
    assert_eq!(rows.log_lookups().len(), 1);

    *rows.window.lock().expect("the fake window row lock") = Some(window_spend_row(start, "3"));
    assert_eq!(db_spend(&counter, start).await, Ok(Some(decimal("3"))));
    assert_eq!(rows.window_lookups().len(), 1);
    assert_eq!(rows.log_lookups().len(), 1);

    let stale = naive("2026-11-17T00:00:00");
    *rows.window.lock().expect("the fake window row lock") = Some(window_spend_row(stale, "3"));
    assert_eq!(db_spend(&counter, start).await, Ok(Some(decimal("8"))));
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
    let rows = Arc::new(FakeRows::empty());
    *rows.logs.lock().expect("the fake logs lock") = vec![mapping_sum("7.5"), mapping_sum("100")];
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    assert_eq!(logs_spend(&counter, start).await, Ok(Some(decimal("7.5"))));
    assert_eq!(
        rows.log_lookups(),
        vec![(
            WindowEntity::Key,
            "k1".to_owned(),
            utc("2026-11-18T00:00:00Z")
        )]
    );
    let team_first = counter
        .window_from_spend_logs(&CounterEntityType::Team, "t1", start)
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

    *rows.logs.lock().expect("the fake logs lock") = Vec::new();
    assert_eq!(logs_spend(&counter, start).await, Ok(Some(Decimal::ZERO)));
    let unsupported = counter
        .window_from_spend_logs(&CounterEntityType::Tag, "tag", start)
        .await;
    assert_eq!(unsupported, Ok(None));
    let rowless = reseed_with(None, services(25), 5.0);
    assert_eq!(logs_spend(&rowless, start).await, Ok(None));

    rows.logs_fail.store(true, Ordering::Relaxed);
    assert_eq!(logs_spend(&counter, start).await, Ok(None));
}

#[tokio::test]
async fn query_failure_falls_back_but_conversion_propagates() {
    let logs = Logs::capture();
    let start = aware("2026-11-18T00:00:00Z");
    let rows = Arc::new(FakeRows::empty());
    *rows.window.lock().expect("the fake window row lock") =
        Some(window_spend_row(start, "\"abc\""));
    *rows.logs.lock().expect("the fake logs lock") = vec![mapping_sum("\"abc\"")];
    let counter = reseed_with(Some(Arc::clone(&rows)), services(25), 5.0);
    assert_eq!(table_spend(&counter, start).await, out_of_range());
    assert_eq!(logs_spend(&counter, start).await, out_of_range());
    assert_eq!(db_spend(&counter, start).await, out_of_range());

    rows.window_fails.store(true, Ordering::Relaxed);
    rows.logs_fail.store(true, Ordering::Relaxed);
    assert_eq!(table_spend(&counter, start).await, Ok(None));
    assert_eq!(logs_spend(&counter, start).await, Ok(None));
    assert!(logs.contains("window_from_table: failed"));
    assert!(logs.contains("window_from_spend_logs: failed"));

    rows.window_fails.store(false, Ordering::Relaxed);
    rows.logs_fail.store(false, Ordering::Relaxed);
    *rows.window.lock().expect("the fake window row lock") =
        Some(window_spend_row(naive("2026-11-17T00:00:00"), "20"));
    *rows.logs.lock().expect("the fake logs lock") = vec![mapping_sum("8")];
    assert_eq!(db_spend(&counter, start).await, Ok(Some(decimal("8"))));
}
