#![cfg(test)]

use axum::http::StatusCode;
use betterlite_lens::models::job::JobStatus;
use betterlite_lens::models::messages::Claim;
use chrono::{DateTime, TimeDelta, Utc};

use crate::lens_access::tests::support::{finding, job, now};
use crate::lens_worker_routes::tests::support::{
    Harness, authorized, base_lens, running_job, team_worker, worker_call, worker_fixture,
    worker_without_key,
};

const CLAIM: &str = "/lens/worker/claim?protocol_version=2";
const UPGRADE: &str = "Upgrade the Lens worker using the current Connect worker command";
const NO_KEY: &str = "Assign an analysis key to this worker in Lens setup";
const NO_DATABASE: &str = "Lens needs a connected Postgres database";

fn shifted(delta: TimeDelta) -> DateTime<Utc> {
    now()
        .checked_add_signed(delta)
        .expect("a representable instant")
}

#[tokio::test]
async fn claim_without_a_protocol_version_is_409_upgrade() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let reply = worker_call(
        harness.app(),
        authorized("POST", "/lens/worker/claim", None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), UPGRADE);
    assert!(harness.database.heartbeats().is_empty());
}

#[tokio::test]
async fn claim_with_another_protocol_version_is_409_upgrade() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let reply = worker_call(
        harness.app(),
        authorized("POST", "/lens/worker/claim?protocol_version=3", None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), UPGRADE);
}

#[tokio::test]
async fn claim_with_a_non_integer_protocol_version_is_422() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let reply = worker_call(
        harness.app(),
        authorized("POST", "/lens/worker/claim?protocol_version=abc", None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "int_parsing");
    assert_eq!(issues[0].loc, ["query", "protocol_version"]);
}

#[tokio::test]
async fn a_repeated_protocol_version_uses_the_last_value() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let reply = worker_call(
        harness.app(),
        authorized(
            "POST",
            "/lens/worker/claim?protocol_version=3&protocol_version=2",
            None,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"null");
}

#[tokio::test]
async fn claim_without_an_analysis_key_is_409() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_without_key());
    let reply = worker_call(harness.app(), authorized("POST", CLAIM, None)).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.detail(), NO_KEY);
    assert!(harness.database.heartbeats().is_empty());
}

#[tokio::test]
async fn claim_without_a_database_is_503_before_the_query() {
    let harness = Harness::new().without_database();
    let reply = worker_call(
        harness.app(),
        authorized("POST", "/lens/worker/claim?protocol_version=abc", None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.detail(), NO_DATABASE);
}

#[tokio::test]
async fn claim_with_nothing_to_claim_returns_null_and_heartbeats() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let reply = worker_call(harness.app(), authorized("POST", CLAIM, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"null");
    assert_eq!(harness.database.heartbeats(), ["2026-05-01T12:00:00+00:00"]);
    assert_eq!(harness.database.worker("w1").last_seen, now());
}

#[tokio::test]
async fn claim_writes_the_heartbeat_before_it_reads_any_lens() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(base_lens("l1"));
    let reply = worker_call(harness.app(), authorized("POST", CLAIM, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        harness.database.events().first().copied(),
        Some("heartbeat")
    );
    assert!(harness.database.events().contains(&"update"));
}

#[tokio::test]
async fn claim_heartbeats_with_microseconds_when_the_clock_has_them() {
    let instant: DateTime<Utc> = "2026-05-01T12:00:00.123456Z"
        .parse()
        .expect("a fixed instant");
    let harness = Harness::new().at(instant);
    harness.database.seed_worker(worker_fixture());
    let reply = worker_call(harness.app(), authorized("POST", CLAIM, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        harness.database.heartbeats(),
        ["2026-05-01T12:00:00.123456+00:00"]
    );
}

#[tokio::test]
async fn claim_takes_a_due_lens() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut stored = base_lens("l1");
    stored.findings = vec![finding("f1")];
    harness.database.seed_lens(stored);
    let reply = worker_call(harness.app(), authorized("POST", CLAIM, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let parsed: Option<Claim> = reply.json();
    let claimed = parsed.expect("a claim");
    assert_eq!(claimed.lens_id, "l1");
    assert_eq!(claimed.job.id, "id-1");
    assert_eq!(claimed.job.status, JobStatus::Running);
    assert_eq!(claimed.job.worker_id.as_deref(), Some("w1"));
    assert_eq!(
        claimed.job.lease_until,
        Some(shifted(TimeDelta::minutes(5)))
    );
    assert_eq!(claimed.job.attempts, 1);
    assert_eq!(claimed.job.stage, "Collecting executions");
    assert_eq!(claimed.job.start, shifted(TimeDelta::hours(-24)));
    assert_eq!(claimed.job.end, shifted(TimeDelta::minutes(-2)));
    assert_eq!(claimed.findings, vec![finding("f1")]);
    let after = harness.database.lens("l1");
    assert_eq!(after.jobs[0].status, JobStatus::Running);
    assert_eq!(after.version, 1);
}

#[tokio::test]
async fn claim_takes_a_queued_job() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut stored = base_lens("l1");
    stored.jobs = vec![job("j1", JobStatus::Queued)];
    harness.database.seed_lens(stored);
    let reply = worker_call(harness.app(), authorized("POST", CLAIM, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let parsed: Option<Claim> = reply.json();
    let claimed = parsed.expect("a claim");
    assert_eq!(claimed.job.id, "j1");
    assert_eq!(claimed.job.worker_id.as_deref(), Some("w1"));
    assert_eq!(claimed.job.attempts, 1);
}

#[tokio::test]
async fn claim_skips_a_lens_in_another_team() {
    let harness = Harness::new();
    harness.database.seed_worker(team_worker());
    harness.database.seed_lens(base_lens("l1"));
    let reply = worker_call(harness.app(), authorized("POST", CLAIM, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"null");
    assert!(harness.database.lens("l1").jobs.is_empty());
}

#[tokio::test]
async fn claim_skips_a_disabled_lens() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut stored = base_lens("l1");
    stored.settings.enabled = false;
    harness.database.seed_lens(stored);
    let reply = worker_call(harness.app(), authorized("POST", CLAIM, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"null");
    assert!(harness.database.lens("l1").jobs.is_empty());
}

#[tokio::test]
async fn claim_skips_a_lens_that_is_not_due() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut stored = base_lens("l1");
    stored.next_run_at = shifted(TimeDelta::hours(1));
    harness.database.seed_lens(stored);
    let reply = worker_call(harness.app(), authorized("POST", CLAIM, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"null");
    assert!(harness.database.lens("l1").jobs.is_empty());
}

#[tokio::test]
async fn claim_skips_a_live_claim() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut stored = base_lens("l1");
    stored.jobs = vec![running_job("j1", "w2", shifted(TimeDelta::minutes(5)))];
    harness.database.seed_lens(stored);
    let reply = worker_call(harness.app(), authorized("POST", CLAIM, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"null");
    let after = harness.database.lens("l1");
    assert_eq!(after.jobs[0].worker_id.as_deref(), Some("w2"));
    assert_eq!(after.jobs[0].attempts, 0);
}

#[tokio::test]
async fn claim_does_not_claim_a_live_job_of_its_own() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut stored = base_lens("l1");
    stored.jobs = vec![running_job("j1", "w1", shifted(TimeDelta::minutes(5)))];
    harness.database.seed_lens(stored);
    let reply = worker_call(harness.app(), authorized("POST", CLAIM, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"null");
}

#[tokio::test]
async fn claim_takes_over_an_expired_claim() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut stored = base_lens("l1");
    stored.jobs = vec![running_job("j1", "w2", shifted(TimeDelta::seconds(-1)))];
    harness.database.seed_lens(stored);
    let reply = worker_call(harness.app(), authorized("POST", CLAIM, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let parsed: Option<Claim> = reply.json();
    let claimed = parsed.expect("a takeover claim");
    assert_eq!(claimed.job.id, "j1");
    assert_eq!(claimed.job.worker_id.as_deref(), Some("w1"));
    assert_eq!(claimed.job.attempts, 1);
    assert_eq!(
        claimed.job.lease_until,
        Some(shifted(TimeDelta::minutes(5)))
    );
}

// docs/deviations/B4505.yaml: the run window of a clock at the calendar's edge is dropped, not raised
#[tokio::test]
async fn claim_answers_null_when_the_run_window_overflows() {
    let instant = DateTime::<Utc>::MIN_UTC
        .checked_add_signed(TimeDelta::minutes(1))
        .expect("a representable instant");
    let harness = Harness::new().at(instant);
    harness.database.seed_worker(worker_fixture());
    let mut stored = base_lens("l1");
    stored.next_run_at = instant;
    harness.database.seed_lens(stored);
    let reply = worker_call(harness.app(), authorized("POST", CLAIM, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"null");
    assert!(harness.database.lens("l1").jobs.is_empty());
}

// docs/deviations/B4505.yaml: the lease instant overflows, so the queued job stays queued and no claim comes back
#[tokio::test]
async fn claim_answers_null_when_the_lease_instant_overflows() {
    let instant = DateTime::<Utc>::MAX_UTC
        .checked_sub_signed(TimeDelta::minutes(1))
        .expect("a representable instant");
    let harness = Harness::new().at(instant);
    harness.database.seed_worker(worker_fixture());
    harness.database.seed_lens(base_lens("l1"));
    let reply = worker_call(harness.app(), authorized("POST", CLAIM, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"null");
    let after = harness.database.lens("l1");
    assert_eq!(after.jobs[0].status, JobStatus::Queued);
}

// a give-up exposes the next active job, which is only claimed when it is running under this worker
#[tokio::test]
async fn claim_does_not_claim_the_job_a_give_up_exposes() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut stored = base_lens("l1");
    let mut exhausted = running_job("j1", "w2", shifted(TimeDelta::seconds(-1)));
    exhausted.attempts = 3;
    stored.jobs = vec![
        exhausted,
        running_job("j2", "w2", shifted(TimeDelta::minutes(5))),
    ];
    harness.database.seed_lens(stored);
    let reply = worker_call(harness.app(), authorized("POST", CLAIM, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, b"null");
    let after = harness.database.lens("l1");
    assert_eq!(after.jobs[0].status, JobStatus::Failed);
    assert_eq!(after.jobs[0].error, "Worker disconnected repeatedly");
    assert_eq!(after.jobs[1].worker_id.as_deref(), Some("w2"));
}

#[tokio::test]
async fn claim_returns_the_first_claimable_lens() {
    let harness = Harness::new();
    harness.database.seed_worker(worker_fixture());
    let mut later = base_lens("l1");
    later.next_run_at = shifted(TimeDelta::hours(1));
    harness.database.seed_lens(later);
    harness.database.seed_lens(base_lens("l2"));
    let reply = worker_call(harness.app(), authorized("POST", CLAIM, None)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let parsed: Option<Claim> = reply.json();
    let claimed = parsed.expect("the second lens is claimable");
    assert_eq!(claimed.lens_id, "l2");
    assert_eq!(claimed.job.id, "id-2");
}
