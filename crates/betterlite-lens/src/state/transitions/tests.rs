#![cfg(test)]

use chrono::{DateTime, TimeDelta, Utc};
use rstest::rstest;
use rust_decimal::Decimal;

use crate::models::job::{JobStatus, Lens, Worker};
use crate::models::messages::RunRequest;
use crate::models::settings::Scope;
use crate::state::transitions::{can_access, claim_job, queue_job, renew_budget};

fn lens() -> Lens {
    serde_json::from_str(include_str!("../fixtures/lens.json")).unwrap()
}

fn worker() -> Worker {
    serde_json::from_str(include_str!("../fixtures/worker.json")).unwrap()
}

fn at(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

fn scope(team_id: &str, api_key_hash: &str, all_teams: bool) -> Scope {
    Scope {
        team_id: team_id.to_owned(),
        api_key_hash: api_key_hash.to_owned(),
        all_teams,
    }
}

#[rstest]
#[case::same_team(scope("team-a", "k1", false), scope("team-a", "k2", false), true)]
#[case::different_team(scope("team-a", "", false), scope("team-b", "", false), false)]
#[case::admin_sees_all(scope("", "", true), scope("team-b", "k2", false), true)]
#[case::shared_lens_needs_admin(scope("team-a", "", false), scope("team-a", "", true), false)]
#[case::same_key_without_team(scope("", "k1", false), scope("", "k1", false), true)]
#[case::other_key_without_team(scope("", "k1", false), scope("", "k2", false), false)]
fn access_follows_team_then_key(
    #[case] viewer: Scope,
    #[case] target: Scope,
    #[case] allowed: bool,
) {
    assert_eq!(can_access(&viewer, &target), allowed);
}

#[rstest]
#[case::lens_lookback(None, 24)]
#[case::request_lookback(Some(6), 6)]
fn queues_one_job_that_stops_two_minutes_back(
    #[case] override_hours: Option<u32>,
    #[case] hours: i64,
) {
    let now = at("2026-01-15T10:00:00Z");
    let request = RunRequest {
        settings: None,
        lookback_hours: override_hours,
    };
    let queued = queue_job(&lens(), now, "job-1".to_owned(), &request).unwrap();
    let job = &queued.jobs[0];
    assert_eq!(queued.jobs.len(), 1);
    assert_eq!(
        (job.id.as_str(), job.status, job.stage.as_str()),
        ("job-1", JobStatus::Queued, "Queued")
    );
    assert_eq!(
        job.start,
        now.checked_sub_signed(TimeDelta::hours(hours)).unwrap()
    );
    assert_eq!(job.end, at("2026-01-15T09:58:00Z"));
    assert_eq!((job.revision, job.attempts, job.created_at), (4, 0, now));
}

#[test]
fn queues_with_the_requested_settings_and_keeps_a_live_job() {
    let now = at("2026-01-15T10:00:00Z");
    let mut settings = lens().settings;
    settings.lookback_hours = 48;
    let request = RunRequest {
        settings: Some(settings.clone()),
        lookback_hours: None,
    };
    let queued = queue_job(&lens(), now, "job-1".to_owned(), &request).unwrap();
    assert_eq!(queued.jobs[0].settings, settings);
    assert_eq!(queued.jobs[0].start, at("2026-01-13T10:00:00Z"));
    let again = queue_job(&queued, now, "job-2".to_owned(), &RunRequest::default()).unwrap();
    assert_eq!(again, queued);
}

#[test]
fn claims_a_queued_job_with_a_five_minute_lease() {
    let now = at("2026-01-15T10:00:00Z");
    let queued = queue_job(&lens(), now, "job-1".to_owned(), &RunRequest::default()).unwrap();
    let claimed = claim_job(&queued, &worker(), now).unwrap();
    let job = &claimed.jobs[0];
    assert_eq!(
        (job.status, job.stage.as_str()),
        (JobStatus::Running, "Collecting executions")
    );
    assert_eq!(
        (job.worker_id.as_deref(), job.attempts),
        (Some("worker-1"), 1)
    );
    assert_eq!(job.lease_until, Some(at("2026-01-15T10:05:00Z")));
    assert_eq!(
        claim_job(&claimed, &worker(), at("2026-01-15T10:04:59Z")).unwrap(),
        claimed
    );
    let reclaimed = claim_job(&claimed, &worker(), at("2026-01-15T10:06:00Z")).unwrap();
    assert_eq!(reclaimed.jobs[0].attempts, 2);
    assert_eq!(
        reclaimed.jobs[0].lease_until,
        Some(at("2026-01-15T10:11:00Z"))
    );
}

#[test]
fn fails_the_job_after_three_attempts() {
    let now = at("2026-01-15T10:00:00Z");
    let mut queued = queue_job(&lens(), now, "job-1".to_owned(), &RunRequest::default()).unwrap();
    queued.jobs[0].attempts = 3;
    let failed = claim_job(&queued, &worker(), now).unwrap();
    let job = &failed.jobs[0];
    assert_eq!(
        (job.status, job.stage.as_str()),
        (JobStatus::Failed, "Failed")
    );
    assert_eq!(job.error, "Worker disconnected repeatedly");
    assert_eq!(
        (job.finished_at, job.attempts, job.worker_id.as_deref()),
        (Some(now), 3, None)
    );
    assert_eq!(failed.next_run_at, at("2026-01-15T10:15:00Z"));
}

#[test]
fn leaves_the_lens_alone_without_access_or_a_job() {
    let now = at("2026-01-15T10:00:00Z");
    assert_eq!(claim_job(&lens(), &worker(), now).unwrap(), lens());
    let queued = queue_job(&lens(), now, "job-1".to_owned(), &RunRequest::default()).unwrap();
    let mut outsider = worker();
    outsider.scope.team_id = "team-b".to_owned();
    assert_eq!(claim_job(&queued, &outsider, now).unwrap(), queued);
}

#[rstest]
#[case::next_month("2026-02-01T00:00:00Z", "2026-02", Decimal::ZERO)]
#[case::same_month("2026-01-31T23:59:59Z", "2026-01", Decimal::new(125, 1))]
fn renews_the_budget_at_the_calendar_month(
    #[case] now: &str,
    #[case] month: &str,
    #[case] spent: Decimal,
) {
    let renewed = renew_budget(&lens(), at(now));
    assert_eq!(
        (renewed.budget_month.as_str(), renewed.spent),
        (month, spent)
    );
}
