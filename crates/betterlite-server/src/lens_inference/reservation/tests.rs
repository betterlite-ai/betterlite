#![cfg(test)]

use std::future::ready;
use std::str::FromStr as _;

use betterlite_db::daily_spend_sql::bind::SqlBind;
use betterlite_db::lens_repository::database::{Database, ExecuteFuture, QueryFuture};
use betterlite_db::lens_repository::repository::LensRepository;
use betterlite_lens::models::job::Lens;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::{ReservationError, ReserveRequest, reserve, reserve_transform};

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

fn amount(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn timestamp(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

fn lens(job_status: &str, job_worker: &str, lease: &str, spent: &str, budget: &str) -> Lens {
    let json = [
        "{\"id\":\"lens-1\",\"scope\":{\"team_id\":\"team-1\"},",
        "\"settings\":{\"name\":\"n\",\"model\":\"lens-model\",\"monthly_budget\":",
        budget,
        "},\"created_at\":\"2026-01-10T08:00:00Z\",\"next_run_at\":\"2026-02-01T00:00:00Z\",",
        "\"budget_month\":\"2026-02\",\"spent\":",
        spent,
        ",\"jobs\":[{\"id\":\"job-1\",",
        "\"status\":\"",
        job_status,
        "\",\"created_at\":\"2026-01-10T08:00:00Z\",",
        "\"start\":\"2026-01-10T00:00:00Z\",\"end\":\"2026-01-10T08:00:00Z\",",
        "\"settings\":{\"name\":\"n\",\"model\":\"lens-model\"},\"revision\":1,",
        "\"worker_id\":\"",
        job_worker,
        "\",\"lease_until\":\"",
        lease,
        "\",\"cost\":0.5}]}",
    ]
    .concat();
    serde_json::from_str(&json).unwrap()
}

fn request(lease_now: &str) -> ReserveRequest<'static> {
    ReserveRequest {
        lens_id: "lens-1",
        job_id: "job-1",
        worker_id: "worker-1",
        month_now: timestamp("2026-02-01T00:00:00Z"),
        lease_now: timestamp(lease_now),
        estimate: amount(ESTIMATE),
    }
}

#[test]
fn a_held_job_with_room_reserves_the_estimate() {
    let lens = lens(
        "running",
        "worker-1",
        "2100-01-01T00:00:00Z",
        "1.5",
        "10000.0",
    );
    let updated = reserve_transform(&lens, &request("2026-02-01T00:00:01Z")).unwrap();
    assert_eq!(updated.spent, amount("4793.0"));
    assert_eq!(updated.jobs[0].cost, amount("4792.0"));
}

#[test]
fn a_queued_job_is_current_too() {
    let lens = lens("queued", "worker-1", "2100-01-01T00:00:00Z", "0", "10000.0");
    let updated = reserve_transform(&lens, &request("2026-02-01T00:00:00Z")).unwrap();
    assert_eq!(updated.spent, amount(ESTIMATE));
}

#[test]
fn a_completed_job_is_not_current() {
    let lens = lens(
        "completed",
        "worker-1",
        "2100-01-01T00:00:00Z",
        "0",
        "10000.0",
    );
    let error = reserve_transform(&lens, &request("2026-02-01T00:00:00Z")).unwrap_err();
    assert!(matches!(error, ReservationError::JobUnavailable));
    assert_eq!(error.status(), 409);
}

#[test]
fn a_spend_that_lands_exactly_on_the_budget_fits() {
    let lens = lens("running", "worker-1", "2100-01-01T00:00:00Z", "0", ESTIMATE);
    let updated = reserve_transform(&lens, &request("2026-02-01T00:00:00Z")).unwrap();
    assert_eq!(updated.spent, amount(ESTIMATE));
}

#[test]
fn a_spend_over_the_budget_is_refused() {
    let lens = lens("running", "worker-1", "2100-01-01T00:00:00Z", "0", "4791.4");
    let error = reserve_transform(&lens, &request("2026-02-01T00:00:00Z")).unwrap_err();
    assert!(matches!(error, ReservationError::BudgetReached));
    assert_eq!(error.status(), 402);
}

#[test]
fn a_lease_that_expires_now_is_expired() {
    let lens = lens(
        "running",
        "worker-1",
        "2026-02-01T00:00:00Z",
        "0",
        "10000.0",
    );
    let error = reserve_transform(&lens, &request("2026-02-01T00:00:00Z")).unwrap_err();
    assert!(matches!(error, ReservationError::JobUnavailable));
}

#[test]
fn a_lease_a_second_ahead_still_holds() {
    let lens = lens(
        "running",
        "worker-1",
        "2026-02-01T00:00:01Z",
        "0",
        "10000.0",
    );
    let updated = reserve_transform(&lens, &request("2026-02-01T00:00:00Z")).unwrap();
    assert_eq!(updated.spent, amount(ESTIMATE));
}

#[test]
fn a_job_claimed_by_another_worker_is_refused() {
    let lens = lens(
        "running",
        "worker-2",
        "2100-01-01T00:00:00Z",
        "0",
        "10000.0",
    );
    let error = reserve_transform(&lens, &request("2026-02-01T00:00:00Z")).unwrap_err();
    assert!(matches!(error, ReservationError::JobUnavailable));
}

#[test]
fn a_job_without_a_lease_is_refused() {
    let mut unleased = lens(
        "running",
        "worker-1",
        "2100-01-01T00:00:00Z",
        "0",
        "10000.0",
    );
    unleased.jobs[0].lease_until = None;
    let error = reserve_transform(&unleased, &request("2026-02-01T00:00:00Z")).unwrap_err();
    assert!(matches!(error, ReservationError::JobUnavailable));
}

#[test]
fn a_new_month_starts_from_zero_spend() {
    let mut lens = lens(
        "running",
        "worker-1",
        "2100-01-01T00:00:00Z",
        "9000.0",
        "10000.0",
    );
    lens.budget_month = "2026-01".to_owned();
    let updated = reserve_transform(&lens, &request("2026-02-01T00:00:00Z")).unwrap();
    assert_eq!(updated.budget_month, "2026-02");
    assert_eq!(updated.spent, amount(ESTIMATE));
}

#[test]
fn a_reserve_outside_the_decimal_range_is_an_overflow() {
    let mut huge = lens(
        "running",
        "worker-1",
        "2100-01-01T00:00:00Z",
        "0",
        "10000.0",
    );
    huge.spent = Decimal::MAX;
    let error = reserve_transform(&huge, &request("2026-02-01T00:00:00Z")).unwrap_err();
    assert!(matches!(error, ReservationError::Overflow));
    assert_eq!(error.status(), 500);
}

#[tokio::test]
async fn a_lens_that_cannot_be_written_is_not_reserved() {
    let repository = LensRepository { database: Empty };
    let error = reserve(&repository, &request("2026-02-01T00:00:00Z"))
        .await
        .unwrap_err();
    assert!(matches!(error, ReservationError::NotReserved));
    assert_eq!(error.status(), 409);
    assert_eq!(error.to_string(), "Could not reserve analysis budget");
}
