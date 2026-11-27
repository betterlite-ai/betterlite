#![cfg(test)]

use std::future::ready;
use std::str::FromStr as _;

use betterlite_db::daily_spend_sql::bind::SqlBind;
use betterlite_db::lens_repository::database::{Database, ExecuteFuture, QueryFuture, Row};
use betterlite_db::lens_repository::repository::LensRepository;
use betterlite_lens::models::job::Lens;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::{SettleRequest, SettlementError, settle, settle_transform};

const ESTIMATE: &str = "4791.5";

struct Empty;

impl Database for Empty {
    fn query_raw<'database>(
        &'database self,
        _query: &'database str,
        _args: &'database [SqlBind],
    ) -> QueryFuture<'database> {
        Box::pin(ready(Ok(Vec::new())))
    }

    fn execute_raw<'database>(
        &'database self,
        _query: &'database str,
        _args: &'database [SqlBind],
    ) -> ExecuteFuture<'database> {
        Box::pin(ready(Ok(0)))
    }
}

struct Corrupt;

impl Database for Corrupt {
    fn query_raw<'database>(
        &'database self,
        _query: &'database str,
        _args: &'database [SqlBind],
    ) -> QueryFuture<'database> {
        let rows = vec![Row {
            data: "not a lens".to_owned(),
        }];
        Box::pin(ready(Ok(rows)))
    }

    fn execute_raw<'database>(
        &'database self,
        _query: &'database str,
        _args: &'database [SqlBind],
    ) -> ExecuteFuture<'database> {
        Box::pin(ready(Ok(0)))
    }
}

fn amount(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn timestamp(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

fn lens(month: &str, spent: &str, job_id: &str, job_cost: &str) -> Lens {
    let json = format!(concat!(
        "{{\"id\":\"lens-1\",\"scope\":{{\"team_id\":\"team-1\"}},",
        "\"settings\":{{\"name\":\"n\",\"model\":\"lens-model\",\"monthly_budget\":10000.0}},",
        "\"created_at\":\"2026-01-10T08:00:00Z\",\"next_run_at\":\"2026-02-01T00:00:00Z\",",
        "\"budget_month\":\"{month}\",\"spent\":{spent},\"jobs\":[{{\"id\":\"{job_id}\",",
        "\"status\":\"running\",\"created_at\":\"2026-01-10T08:00:00Z\",",
        "\"start\":\"2026-01-10T00:00:00Z\",\"end\":\"2026-01-10T08:00:00Z\",",
        "\"settings\":{{\"name\":\"n\",\"model\":\"lens-model\"}},\"revision\":1,",
        "\"worker_id\":\"worker-1\",\"lease_until\":\"2100-01-01T00:00:00Z\",",
        "\"cost\":{job_cost}}}]}}"
    ));
    serde_json::from_str(&json).unwrap()
}

fn request(job_id: &'static str, cost: &str) -> SettleRequest<'static> {
    SettleRequest {
        lens_id: "lens-1",
        job_id,
        month_now: timestamp("2026-02-01T00:00:00Z"),
        estimate: amount(ESTIMATE),
        cost: amount(cost),
    }
}

#[test]
fn the_estimate_comes_off_and_the_billed_cost_goes_on() {
    let updated = settle_transform(
        lens("2026-02", "10000.0", "job-1", "4791.5"),
        &request("job-1", "4.0"),
    )
    .unwrap();
    assert_eq!(updated.spent, amount("5212.5"));
    assert_eq!(updated.jobs[0].cost, amount("4.0"));
}

#[test]
fn another_month_keeps_the_spent_total() {
    let updated = settle_transform(
        lens("2026-01", "10000.0", "job-1", "4791.5"),
        &request("job-1", "4.0"),
    )
    .unwrap();
    assert_eq!(updated.spent, amount("10000"));
    assert_eq!(updated.jobs[0].cost, amount("4.0"));
}

#[test]
fn a_spend_under_the_estimate_clamps_at_zero() {
    let updated = settle_transform(
        lens("2026-02", "10.0", "job-1", "4791.5"),
        &request("job-1", "1.0"),
    )
    .unwrap();
    assert_eq!(updated.spent, Decimal::ZERO);
    assert_eq!(updated.jobs[0].cost, amount("1.0"));
}

#[test]
fn a_job_cost_under_the_estimate_clamps_at_zero() {
    let updated = settle_transform(
        lens("2026-02", "10000.0", "job-1", "0.5"),
        &request("job-1", "4.0"),
    )
    .unwrap();
    assert_eq!(updated.spent, amount("5212.5"));
    assert_eq!(updated.jobs[0].cost, Decimal::ZERO);
}

#[test]
fn a_missing_job_still_settles_the_spent_total() {
    let updated = settle_transform(
        lens("2026-02", "10000.0", "job-other", "0.5"),
        &request("job-1", "4.0"),
    )
    .unwrap();
    assert_eq!(updated.spent, amount("5212.5"));
    assert_eq!(updated.jobs[0].cost, amount("0.5"));
}

#[test]
fn a_missing_job_in_another_month_changes_nothing() {
    let before = lens("2026-01", "10000.0", "job-other", "0.5");
    let updated = settle_transform(before.clone(), &request("job-1", "4.0")).unwrap();
    assert_eq!(updated, before);
}

#[test]
fn a_spend_outside_the_decimal_range_is_an_overflow() {
    let mut huge = lens("2026-02", "0", "job-1", "0.5");
    huge.spent = Decimal::MIN;
    let error = settle_transform(huge, &request("job-1", "4.0")).unwrap_err();
    assert!(matches!(error, SettlementError::Overflow));
    assert_eq!(
        error.to_string(),
        "the settled lens spend is outside the decimal range"
    );
}

#[tokio::test]
async fn a_stored_lens_that_cannot_be_read_is_a_repository_error() {
    let repository = LensRepository { database: Corrupt };
    let error = settle(&repository, &request("job-1", "4.0"))
        .await
        .unwrap_err();
    assert!(matches!(error, SettlementError::Repository(_)));
}

#[tokio::test]
async fn a_lens_that_cannot_be_written_is_still_settled() {
    let repository = LensRepository { database: Empty };
    settle(&repository, &request("job-1", "4.0")).await.unwrap();
}
