#![cfg(test)]

use std::sync::Arc;

use chrono::NaiveTime;
use rstest::rstest;
use tokio_util::task::TaskTracker;

use crate::budget_sweep::BudgetSweep;
use crate::budget_sweep::phase::{PhaseRunner as _, ResetPhase};
use crate::budget_sweep::production::production;
use crate::budget_sweep::settings::BudgetResetSettings;
use crate::postgres_store::session::PostgresStore;
use crate::queues::pod_lock::NoPodLockEventSink;
use crate::queues::pod_lock::PodLockManager;

use super::support::{
    RecordingLeaseCache, RecordingPhases, TestSnapshot, assembly_inputs, captured_logs, lazy_store,
    pod_id,
};

#[test]
fn default_settings_are_utc_and_midnight() {
    let settings = BudgetResetSettings::default();
    assert_eq!(settings.timezone, "UTC");
    assert_eq!(settings.reset_time_of_day, NaiveTime::MIN);
}

#[tokio::test]
async fn explicit_settings_and_the_shared_database_reach_the_real_adapter() {
    let settings = BudgetResetSettings {
        timezone: "Europe/Paris".to_owned(),
        reset_time_of_day: NaiveTime::from_hms_opt(12, 30, 0).unwrap(),
    };
    let inputs = assembly_inputs(
        lazy_store(),
        Some(settings.clone()),
        None,
        TaskTracker::new(),
    );
    let snapshot = TestSnapshot::new(BudgetResetSettings::default());
    let sweep = production(inputs, &snapshot);
    assert_eq!(sweep.settings, settings);
    assert_eq!(sweep.phases.settings, settings);
    assert_eq!(snapshot.calls(), 0);
    sweep.database.pool.as_ref().unwrap().close();
    assert!(sweep.phases.store.pool.as_ref().unwrap().is_closed());
}

#[tokio::test]
async fn missing_settings_use_the_injected_snapshot() {
    let configured = BudgetResetSettings {
        timezone: "Asia/Tokyo".to_owned(),
        reset_time_of_day: NaiveTime::from_hms_opt(3, 15, 0).unwrap(),
    };
    let snapshot = TestSnapshot::new(configured.clone());
    let inputs = assembly_inputs(PostgresStore { pool: None }, None, None, TaskTracker::new());
    let mut sweep = production(inputs, &snapshot);
    assert_eq!(sweep.settings, configured);
    assert_eq!(sweep.phases.settings, configured);
    assert_eq!(snapshot.calls(), 1);
    sweep.reset_budget().await.unwrap();
    assert_eq!(snapshot.calls(), 1);
}

#[tokio::test]
async fn a_missing_database_does_nothing() {
    let cache = Arc::new(RecordingLeaseCache::granting());
    let manager = Arc::new(PodLockManager::new(
        Some(Arc::clone(&cache)),
        60,
        Arc::new(NoPodLockEventSink),
        pod_id(),
    ));
    let phases = RecordingPhases::default();
    let calls = phases.clone();
    let mut sweep = BudgetSweep {
        database: PostgresStore { pool: None },
        settings: BudgetResetSettings::default(),
        lock_manager: Some(manager),
        phases,
        tasks: TaskTracker::new(),
    };
    sweep.reset_budget().await.unwrap();
    assert!(cache.operations().is_empty());
    assert!(calls.phases().is_empty());
}

#[rstest]
#[case::keys(ResetPhase::Keys, Some("reset_budget_keys"))]
#[case::users(ResetPhase::Users, Some("reset_budget_users"))]
#[case::teams(ResetPhase::Teams, Some("reset_budget_teams"))]
#[case::tiers(ResetPhase::Tiers, Some("reset_budget_endusers"))]
#[case::windows(ResetPhase::Windows, None)]
#[tokio::test]
async fn the_real_adapter_maps_every_phase_to_its_lower_service(
    #[case] phase: ResetPhase,
    #[case] call_type: Option<&str>,
) {
    let inputs = assembly_inputs(
        PostgresStore { pool: None },
        Some(BudgetResetSettings::default()),
        None,
        TaskTracker::new(),
    );
    let failures = inputs.ports.failure.clone();
    let mut sweep = production(inputs, &TestSnapshot::new(BudgetResetSettings::default()));
    sweep.phases.run(phase).await.unwrap();
    let expected: Vec<String> = call_type.map(str::to_owned).into_iter().collect();
    assert_eq!(failures.call_types(), expected);
}

#[tokio::test]
async fn the_real_adapter_logs_each_failing_window_source() {
    let inputs = assembly_inputs(
        PostgresStore { pool: None },
        Some(BudgetResetSettings::default()),
        None,
        TaskTracker::new(),
    );
    let failures = inputs.ports.failure.clone();
    let mut sweep = production(inputs, &TestSnapshot::new(BudgetResetSettings::default()));
    let (logs, guard) = captured_logs();
    sweep.phases.run(ResetPhase::Windows).await.unwrap();
    drop(guard);
    let text = logs.text();
    assert!(text.contains("Failed to reset budget windows for keys"));
    assert!(text.contains("Failed to reset budget windows for teams"));
    assert!(failures.call_types().is_empty());
}
