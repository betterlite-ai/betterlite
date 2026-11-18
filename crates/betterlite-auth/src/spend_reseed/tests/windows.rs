use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_db::budget_permission_rows::Timestamp;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{
    FakeRows, Logs, aware, decimal, mapping_sum, naive, reseed, reseed_with, services, spend_json,
    utc, window_spend_row,
};
use crate::budget_counter::CounterEntityType;
use crate::spend_reseed::reseed::SpendCounterReseed;
use crate::spend_reseed::rows::{WindowEntity, WindowSpendRow};
use crate::spend_reseed::values::SpendValueError;
use crate::spend_reseed::windows::WindowQuery;

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
    assert_eq!(user, Ok(None));
    assert_eq!(rows.window_lookups().len(), 2);
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
