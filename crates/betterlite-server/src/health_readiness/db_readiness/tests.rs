#![cfg(test)]

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::health_readiness::Settings as HealthSettings;
use betterlite_db::database_availability::policy::AllowRequestsOnOutage;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_faults::ErrorType;
use rstest::rstest;
use tokio::time::{Instant, advance};

use super::{DbReadiness, DbStatus, StallTracker as _};
use crate::health_readiness::tests::support::{
    FakeProbe, FakeSources, FakeStall, ProbeAnswer, ReconnectAnswer, WorkerCensus, capture_logs,
    db_readiness,
};

struct Fixture {
    probe: Arc<FakeProbe>,
    db: DbReadiness,
}

fn fixture_with(probe: FakeProbe, stalled: bool, settings: &HealthSettings) -> Fixture {
    let shared_probe = Arc::new(probe);
    let stall = Arc::new(FakeStall { stalled });
    let db = db_readiness(
        &shared_probe,
        &stall,
        AllowRequestsOnOutage::Unsupported,
        settings,
    );
    Fixture {
        probe: shared_probe,
        db,
    }
}

fn fixture(probe: FakeProbe) -> Fixture {
    fixture_with(probe, false, &HealthSettings::default())
}

#[tokio::test(start_paused = true)]
async fn db_connected_is_cached_for_fifteen_seconds() {
    let fixture = fixture(FakeProbe::scripted(vec![
        ProbeAnswer::Ok,
        ProbeAnswer::Fail(ErrorType::Data),
    ]));
    let first = fixture.db.check().await;
    assert_eq!(first.status, DbStatus::Connected);
    advance(Duration::from_millis(14_900)).await;
    let cached = fixture.db.check().await;
    assert_eq!(cached.status, DbStatus::Connected);
    assert_eq!(fixture.probe.health_calls.load(Ordering::Acquire), 1);
    // exactly fifteen seconds old is no longer cacheable, as LiteLLM's strict comparison says
    advance(Duration::from_millis(100)).await;
    let expired = fixture.db.check().await;
    assert_eq!(expired.status, DbStatus::Disconnected);
    assert_eq!(fixture.probe.health_calls.load(Ordering::Acquire), 2);
}

#[tokio::test(start_paused = true)]
async fn db_disconnected_is_not_cached() {
    let fixture = fixture(FakeProbe::scripted(vec![
        ProbeAnswer::Fail(ErrorType::Data),
        ProbeAnswer::Fail(ErrorType::Data),
    ]));
    let first = fixture.db.check().await;
    assert_eq!(first.status, DbStatus::Disconnected);
    let second = fixture.db.check().await;
    assert_eq!(second.status, DbStatus::Disconnected);
    assert_eq!(fixture.probe.health_calls.load(Ordering::Acquire), 2);
}

#[tokio::test(start_paused = true)]
async fn db_without_a_database_is_disconnected_and_never_probed() {
    let fixture = fixture(FakeProbe::unconfigured());
    let health = fixture.db.check().await;
    assert_eq!(health.status, DbStatus::Disconnected);
    assert_eq!(fixture.probe.health_calls.load(Ordering::Acquire), 0);
}

#[tokio::test(start_paused = true)]
async fn db_transport_error_reconnects_once_and_reports_connected() {
    let (logs, _guard) = capture_logs(tracing::Level::INFO);
    let probe = FakeProbe::scripted(vec![ProbeAnswer::Fail(ErrorType::Connect), ProbeAnswer::Ok])
        .with_reconnects(vec![ReconnectAnswer::Report(true)]);
    let fixture = fixture(probe);
    let health = fixture.db.check().await;
    assert_eq!(health.status, DbStatus::Connected);
    let requests = fixture.probe.reconnect_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "health_readiness_check");
    assert_eq!(requests[0].1.to_bits(), 2.0_f64.to_bits());
    assert_eq!(requests[0].2.to_bits(), 2.0_f64.to_bits());
    let contents = logs.contents();
    assert!(contents.contains("health_check failed, attempting reconnect"));
    assert!(contents.contains("_db_health_readiness_check: reconnect succeeded"));
}

#[tokio::test(start_paused = true)]
async fn db_reconnect_that_reports_no_attempt_still_rechecks() {
    let probe = FakeProbe::scripted(vec![ProbeAnswer::Fail(ErrorType::Connect), ProbeAnswer::Ok])
        .with_reconnects(vec![ReconnectAnswer::Report(false)]);
    let fixture = fixture(probe);
    let health = fixture.db.check().await;
    assert_eq!(health.status, DbStatus::Connected);
    assert_eq!(fixture.probe.health_calls.load(Ordering::Acquire), 2);
}

#[tokio::test(start_paused = true)]
async fn db_second_check_after_a_reconnect_failure_is_disconnected() {
    let (logs, _guard) = capture_logs(tracing::Level::ERROR);
    let probe = FakeProbe::scripted(vec![
        ProbeAnswer::Fail(ErrorType::Connect),
        ProbeAnswer::Fail(ErrorType::Read),
    ])
    .with_reconnects(vec![ReconnectAnswer::Report(true)]);
    let fixture = fixture(probe);
    let health = fixture.db.check().await;
    assert_eq!(health.status, DbStatus::Disconnected);
    assert!(
        logs.contents()
            .contains("_db_health_readiness_check: reconnect failed")
    );
}

#[tokio::test(start_paused = true)]
async fn db_reconnect_error_is_disconnected() {
    let probe = FakeProbe::scripted(vec![ProbeAnswer::Fail(ErrorType::Connect)])
        .with_reconnects(vec![ReconnectAnswer::Fail(ErrorType::Connect)]);
    let fixture = fixture(probe);
    let health = fixture.db.check().await;
    assert_eq!(health.status, DbStatus::Disconnected);
}

#[tokio::test(start_paused = true)]
async fn db_data_error_does_not_reconnect() {
    let fixture = fixture(FakeProbe::scripted(vec![ProbeAnswer::Fail(
        ErrorType::Data,
    )]));
    let health = fixture.db.check().await;
    assert_eq!(health.status, DbStatus::Disconnected);
    assert!(fixture.probe.reconnect_requests().is_empty());
}

#[tokio::test(start_paused = true)]
async fn db_health_check_timeout_is_disconnected_without_reconnect() {
    let fixture = fixture(FakeProbe::scripted(vec![ProbeAnswer::Hang]));
    let started = Instant::now();
    let health = fixture.db.check().await;
    assert_eq!(health.status, DbStatus::Disconnected);
    assert_eq!(started.elapsed(), Duration::from_secs(2));
    assert!(fixture.probe.reconnect_requests().is_empty());
}

#[tokio::test(start_paused = true)]
async fn db_probe_deadline_returns_the_cached_instant_and_writes_nothing() {
    let probe = FakeProbe::scripted(vec![ProbeAnswer::Fail(ErrorType::Connect), ProbeAnswer::Ok])
        .with_reconnects(vec![ReconnectAnswer::Hang]);
    let fixture = fixture(probe);
    let started = Instant::now();
    let health = fixture.db.check().await;
    assert_eq!(started.elapsed(), Duration::from_secs(4));
    assert_eq!(health.status, DbStatus::Disconnected);
    let recorded = *fixture.probe.last_health_call.lock().unwrap();
    let failed_at = recorded.expect("the health check ran before the reconnect hung");
    assert_eq!(health.last_updated, failed_at);
    let next = fixture.db.check().await;
    assert_eq!(next.status, DbStatus::Connected);
    assert_eq!(fixture.probe.health_calls.load(Ordering::Acquire), 2);
}

#[rstest]
#[case::connected_stays_connected(DbStatus::Connected, false, DbStatus::Connected)]
#[case::connected_over_a_stall_is_stalled(DbStatus::Connected, true, DbStatus::Stalled)]
#[case::disconnected_stays_disconnected(DbStatus::Disconnected, true, DbStatus::Disconnected)]
#[case::not_connected_stays_not_connected(DbStatus::NotConnected, true, DbStatus::NotConnected)]
fn stall_replaces_only_a_connected_status(
    #[case] status: DbStatus,
    #[case] stalled: bool,
    #[case] expected: DbStatus,
) {
    let fixture = fixture_with(FakeProbe::healthy(), stalled, &HealthSettings::default());
    assert_eq!(fixture.db.readiness_db_status(status), expected);
}

#[tokio::test(start_paused = true)]
async fn no_redis_warning_is_off_when_redis_is_configured() {
    let fixture = fixture(FakeProbe::unconfigured());
    let sources = FakeSources::default().with_redis(true);
    assert!(!fixture.db.show_no_redis_warning(&sources).await);
}

#[tokio::test(start_paused = true)]
async fn no_redis_warning_is_off_when_the_flag_disables_it() {
    let db = DbReadiness::new(
        Arc::new(FakeProbe::unconfigured()),
        Arc::new(FakeStall { stalled: false }),
        AllowRequestsOnOutage::Unsupported,
        30.0_f64,
        true,
    );
    assert!(!db.show_no_redis_warning(&FakeSources::default()).await);
}

#[tokio::test(start_paused = true)]
async fn no_redis_warning_stays_on_without_a_database() {
    let fixture = fixture(FakeProbe::unconfigured());
    assert!(
        fixture
            .db
            .show_no_redis_warning(&FakeSources::default())
            .await
    );
}

#[test]
fn stall_tracker_is_not_stalled_before_any_lookup() {
    let tracker = DbLookupStallTracker {
        clock: SystemClock,
        last_hit: Mutex::new(None),
    };
    assert!(!tracker.stalled_within_window(f64::MAX));
}

#[test]
fn stall_tracker_is_stalled_within_the_window_after_a_lookup() {
    let tracker = DbLookupStallTracker {
        clock: SystemClock,
        last_hit: Mutex::new(None),
    };
    tracker.record_hit();
    assert!(tracker.stalled_within_window(60.0));
}

#[rstest]
#[case::one_worker(WorkerCensus::Workers(1), false)]
#[case::two_workers(WorkerCensus::Workers(2), true)]
#[case::failed_census(WorkerCensus::Failed, true)]
#[tokio::test]
async fn no_redis_warning_stays_on_unless_exactly_one_worker(
    #[case] census: WorkerCensus,
    #[case] expected: bool,
) {
    let mut probe = FakeProbe::healthy();
    probe.census = census;
    let fixture = fixture(probe);
    let warning = fixture
        .db
        .show_no_redis_warning(&FakeSources::default())
        .await;
    assert_eq!(warning, expected);
}
