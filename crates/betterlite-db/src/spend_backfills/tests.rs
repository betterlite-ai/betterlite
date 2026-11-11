#![cfg(test)]

use std::error::Error as _;
use std::sync::Mutex;

use chrono::{DateTime, NaiveDateTime, Utc};
use rstest::rstest;
use rust_decimal::Decimal;
use tokio_postgres::Config;

use super::fixtures::SpendLogSeed;
use super::{
    BackfillError, BackfillFuture, BackfillOutcome, BackfillPlan, BackfillStatement,
    ExecutesBackfillStatements, daily_tool_spend, key_total_spend, key_total_spend_from_logs,
    run_plan,
};

mod postgres;
mod scenarios;

fn cutover() -> DateTime<Utc> {
    "2026-07-25T00:00:00Z".parse().expect("a UTC cutover")
}

pub(super) fn daily_plan() -> BackfillPlan {
    BackfillPlan::DailyToolSpend { cutover: cutover() }
}

pub(super) fn money(units: i64, scale: u32) -> Decimal {
    Decimal::new(units, scale)
}

pub(super) fn log_seed(
    request_id: &str,
    spend: Decimal,
    total_tokens: i32,
    start_time: NaiveDateTime,
) -> SpendLogSeed<'_> {
    SpendLogSeed {
        request_id,
        api_key: "sk-test",
        spend,
        total_tokens,
        start_time,
    }
}

fn driver_error() -> tokio_postgres::Error {
    "unknown_option=on"
        .parse::<Config>()
        .expect_err("an unknown connection option")
}

#[derive(Default)]
struct Scripted {
    statements: Mutex<Vec<BackfillStatement>>,
    failure: Option<BackfillStatement>,
}

impl Scripted {
    fn failing_at(statement: BackfillStatement) -> Self {
        Self {
            statements: Mutex::new(Vec::new()),
            failure: Some(statement),
        }
    }

    fn statements(&self) -> Vec<BackfillStatement> {
        self.statements.lock().expect("the statement log").clone()
    }
}

impl ExecutesBackfillStatements for Scripted {
    fn execute_backfill(&self, statement: BackfillStatement) -> BackfillFuture<'_> {
        self.statements
            .lock()
            .expect("the statement log")
            .push(statement);
        let failure = self.failure == Some(statement);
        Box::pin(async move { if failure { Err(driver_error()) } else { Ok(1) } })
    }
}

#[derive(Clone, Copy)]
enum Plan {
    Daily,
    Floor,
    History,
}

fn make_plan(plan: Plan) -> BackfillPlan {
    match plan {
        Plan::Daily => daily_plan(),
        Plan::Floor => BackfillPlan::KeyTotalSpend,
        Plan::History => BackfillPlan::KeyTotalSpendFromLogs,
    }
}

#[tokio::test]
async fn lifetime_at_least_current() {
    let scripted = Scripted::default();
    let outcome = run_plan(&scripted, BackfillPlan::KeyTotalSpend)
        .await
        .expect("the floor plan");
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpend {
            active_keys: 1,
            archived_keys: 1
        }
    );
    assert_eq!(
        scripted.statements(),
        vec![
            BackfillStatement::FloorActiveKeys,
            BackfillStatement::FloorArchivedKeys
        ]
    );
    assert!(key_total_spend::ACTIVE_SQL.contains("SET total_spend = spend"));
    assert!(key_total_spend::ACTIVE_SQL.contains("WHERE total_spend < spend"));
    assert!(key_total_spend::ARCHIVED_SQL.contains("SET total_spend = spend"));
    assert!(key_total_spend::ARCHIVED_SQL.contains("WHERE total_spend < spend"));
}

#[tokio::test]
async fn already_correct_noop() {
    assert!(!key_total_spend::ACTIVE_SQL.contains("<="));
    assert!(!key_total_spend::ARCHIVED_SQL.contains("<="));
    assert!(!key_total_spend::ACTIVE_SQL.contains("updated_at"));
    assert!(!key_total_spend::ARCHIVED_SQL.contains("updated_at"));
    let scripted = Scripted::default();
    let first = run_plan(&scripted, BackfillPlan::KeyTotalSpend)
        .await
        .expect("the first run");
    let second = run_plan(&scripted, BackfillPlan::KeyTotalSpend)
        .await
        .expect("the second run");
    assert_eq!(first, second);
    assert_eq!(scripted.statements().len(), 4);
}

#[tokio::test]
async fn archived_key() {
    let scripted = Scripted::default();
    let outcome = run_plan(&scripted, BackfillPlan::KeyTotalSpend)
        .await
        .expect("the floor plan");
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpend {
            active_keys: 1,
            archived_keys: 1
        }
    );
    assert_eq!(
        scripted.statements().last(),
        Some(&BackfillStatement::FloorArchivedKeys)
    );
    assert!(key_total_spend::ARCHIVED_SQL.contains("LiteLLM_DeletedVerificationToken"));
    assert!(!key_total_spend::ACTIVE_SQL.contains("DeletedVerificationToken"));
}

#[tokio::test]
async fn resetting_key_history() {
    let scripted = Scripted::default();
    let outcome = run_plan(&scripted, BackfillPlan::KeyTotalSpendFromLogs)
        .await
        .expect("the history plan");
    assert_eq!(
        outcome,
        BackfillOutcome::KeyTotalSpendFromLogs {
            active_keys: 1,
            archived_keys: 1
        }
    );
    assert_eq!(
        scripted.statements(),
        vec![
            BackfillStatement::HistoryActiveKeys,
            BackfillStatement::HistoryArchivedKeys
        ]
    );
    assert!(key_total_spend_from_logs::ACTIVE_SQL.contains("SUM(l.spend)"));
    assert!(!key_total_spend_from_logs::ACTIVE_SQL.contains("COALESCE"));
    assert!(key_total_spend_from_logs::ACTIVE_SQL.contains("k2.budget_duration IS NOT NULL"));
    assert!(
        key_total_spend_from_logs::ACTIVE_SQL
            .contains("encode(sha256(convert_to(k2.token, 'UTF8')), 'hex')")
    );
    assert!(key_total_spend_from_logs::ACTIVE_SQL.contains("k.total_spend < s.sum_spend"));
    assert!(key_total_spend_from_logs::ARCHIVED_SQL.contains("SELECT DISTINCT token"));
    assert!(key_total_spend_from_logs::ARCHIVED_SQL.contains("AND (k.budget_duration IS NOT NULL"));
}

#[tokio::test]
async fn daily_tool_aggregation() {
    let scripted = Scripted::default();
    let outcome = run_plan(&scripted, daily_plan())
        .await
        .expect("the daily plan");
    assert_eq!(outcome, BackfillOutcome::DailyToolSpend { buckets: 1 });
    assert_eq!(
        scripted.statements(),
        vec![
            BackfillStatement::SessionTimeZoneUtc,
            BackfillStatement::DailyToolSpend { cutover: cutover() }
        ]
    );
    assert_eq!(daily_tool_spend::SET_TIME_ZONE_SQL, "SET TIME ZONE 'UTC'");
    assert!(daily_tool_spend::INSERT_SQL.contains("to_char(ti.start_time, 'YYYY-MM-DD')"));
    assert!(daily_tool_spend::INSERT_SQL.contains("COALESCE(SUM(sl.spend), 0)"));
    assert!(daily_tool_spend::INSERT_SQL.contains("COALESCE(SUM(sl.total_tokens), 0)"));
    assert!(daily_tool_spend::INSERT_SQL.contains("COUNT(*) AS request_count"));
    assert!(
        daily_tool_spend::INSERT_SQL
            .contains(r#"JOIN "LiteLLM_SpendLogs" sl ON sl.request_id = ti.request_id"#)
    );
    assert!(daily_tool_spend::INSERT_SQL.contains("WHERE ti.start_time < $1::timestamptz"));
    assert!(daily_tool_spend::INSERT_SQL.contains("GROUP BY 1, 2"));
    assert!(daily_tool_spend::INSERT_SQL.contains("ON CONFLICT (date, tool_name) DO NOTHING"));
    assert!(daily_tool_spend::INSERT_SQL.contains("now() AS created_at"));
    assert!(daily_tool_spend::INSERT_SQL.contains("now() AS updated_at"));
}

#[rstest]
#[case::time_zone(
    BackfillStatement::SessionTimeZoneUtc,
    Plan::Daily,
    daily_tool_spend::SESSION_TIME_ZONE_FAILURE,
    1
)]
#[case::daily_insert(
    BackfillStatement::DailyToolSpend { cutover: cutover() },
    Plan::Daily,
    daily_tool_spend::INSERT_FAILURE,
    2
)]
#[case::floor_active(
    BackfillStatement::FloorActiveKeys,
    Plan::Floor,
    key_total_spend::ACTIVE_FAILURE,
    1
)]
#[case::floor_archived(
    BackfillStatement::FloorArchivedKeys,
    Plan::Floor,
    key_total_spend::ARCHIVED_FAILURE,
    2
)]
#[case::history_active(
    BackfillStatement::HistoryActiveKeys,
    Plan::History,
    key_total_spend_from_logs::ACTIVE_FAILURE,
    1
)]
#[case::history_archived(
    BackfillStatement::HistoryArchivedKeys,
    Plan::History,
    key_total_spend_from_logs::ARCHIVED_FAILURE,
    2
)]
#[tokio::test]
async fn statement_failure(
    #[case] failed: BackfillStatement,
    #[case] plan: Plan,
    #[case] label: &'static str,
    #[case] expected_calls: usize,
) {
    let scripted = Scripted::failing_at(failed);
    let error = run_plan(&scripted, make_plan(plan))
        .await
        .expect_err("the failing plan");
    assert!(error.source().is_some());
    let BackfillError::Statement { statement, .. } = error;
    assert_eq!(statement, label);
    assert_eq!(scripted.statements().len(), expected_calls);
    assert_eq!(scripted.statements().last(), Some(&failed));
}

#[tokio::test]
async fn postgres_pinned_rows() {
    postgres::cutover_strict_utc().await;
    postgres::index_time_not_log_time().await;
    postgres::null_sums_zero().await;
    postgres::full_spend_per_tool().await;
    postgres::bucket_conflict_skips_all().await;
    postgres::floor_no_budget_filter().await;
    postgres::floor_null_unchanged().await;
    postgres::own_or_linked_reset().await;
    postgres::empty_duration_is_reset().await;
    postgres::token_or_double_hash().await;
    postgres::archive_distinct_tokens().await;
    postgres::mixed_archive_lifetimes().await;
    postgres::shorter_history_never_lowers().await;
}

#[tokio::test]
async fn postgres_failure_effects() {
    postgres::active_commits_before_archive().await;
    postgres::logs_second_write_failure().await;
}
