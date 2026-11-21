#![cfg(test)]

mod support;

use std::sync::Arc;

use rstest::rstest;
use tokio::sync::Notify;
use tokio_util::task::TaskTracker;

use crate::budget_sweep::BudgetSweep;
use crate::budget_sweep::error::SweepError;
use crate::budget_sweep::phase::ResetPhase;
use crate::budget_sweep::settings::BudgetResetSettings;
use crate::queues::pod_lock::NoPodLockEventSink;
use crate::queues::pod_lock::PodLockManager;

use self::support::{
    LeaseOperation, PhaseEvent, RecordingLeaseCache, RecordingPhases, captured_logs, lazy_store,
    pod_id,
};

const LEASE_KEY: &str = "cronjob_lock:reset_budget_job";
const SKIP_MESSAGE: &str = "another pod holds the reset lease, skipping this tick";
const UNGUARDED_MESSAGE: &str = "could not take the reset lease and no other pod holds it";
const READ_FAILURE_MESSAGE: &str = "could not read the reset lease";

mod sweep_assembly {
    use std::sync::Arc;

    use chrono::NaiveTime;
    use tokio_util::task::TaskTracker;

    use crate::budget_sweep::BudgetSweep;
    use crate::budget_sweep::phase::{PhaseRunner, ResetPhase};
    use crate::budget_sweep::settings::BudgetResetSettings;
    use crate::postgres_store::session::PostgresStore;
    use crate::queues::pod_lock::NoPodLockEventSink;
    use crate::queues::pod_lock::PodLockManager;

    use super::support::{
        RecordingLeaseCache, RecordingPhases, TestSnapshot, assembly_inputs, captured_logs,
        lazy_store, pod_id,
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
        let sweep = BudgetSweep::production(inputs, &snapshot);
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
        let mut sweep = BudgetSweep::production(inputs, &snapshot);
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

    #[tokio::test]
    async fn the_real_adapter_maps_every_phase_to_its_lower_service() {
        let inputs = assembly_inputs(
            PostgresStore { pool: None },
            Some(BudgetResetSettings::default()),
            None,
            TaskTracker::new(),
        );
        let failures = inputs.ports.failure.clone();
        let mut sweep =
            BudgetSweep::production(inputs, &TestSnapshot::new(BudgetResetSettings::default()));
        let expected = [
            (ResetPhase::Keys, "reset_budget_keys"),
            (ResetPhase::Users, "reset_budget_users"),
            (ResetPhase::Teams, "reset_budget_teams"),
            (ResetPhase::Tiers, "reset_budget_endusers"),
        ];
        for (ran, (phase, call_type)) in expected.into_iter().enumerate() {
            sweep.phases.run(phase).await.unwrap();
            let recorded = failures.call_types();
            assert_eq!(recorded.len(), ran + 1);
            assert_eq!(recorded[ran], call_type);
        }
        let (logs, guard) = captured_logs();
        sweep.phases.run(ResetPhase::Windows).await.unwrap();
        drop(guard);
        let text = logs.text();
        assert!(text.contains("Failed to reset budget windows for keys"));
        assert!(text.contains("Failed to reset budget windows for teams"));
        assert_eq!(failures.call_types().len(), 4);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElectionCase {
    AbsentManager,
    ManagerWithoutRedis,
    Acquired,
    Reentrant,
    HeldByOther,
    MissingAfterFailedAcquire,
    EmptyStoredOwnerAfterFailedAcquire,
    UndecodableOwnerAfterFailedAcquire,
    UnreadableAfterFailedAcquire,
}

#[rstest]
#[case::absent_manager(ElectionCase::AbsentManager)]
#[case::manager_without_redis(ElectionCase::ManagerWithoutRedis)]
#[case::acquired(ElectionCase::Acquired)]
#[case::reentrant(ElectionCase::Reentrant)]
#[case::held_by_other(ElectionCase::HeldByOther)]
#[case::missing_after_failed_acquire(ElectionCase::MissingAfterFailedAcquire)]
#[case::empty_stored_owner_after_failed_acquire(ElectionCase::EmptyStoredOwnerAfterFailedAcquire)]
#[case::undecodable_owner_after_failed_acquire(ElectionCase::UndecodableOwnerAfterFailedAcquire)]
#[case::unreadable_after_failed_acquire(ElectionCase::UnreadableAfterFailedAcquire)]
#[tokio::test]
async fn sweep_lease_election(#[case] case: ElectionCase) {
    let pod = pod_id();
    let cache = match case {
        ElectionCase::AbsentManager | ElectionCase::ManagerWithoutRedis => None,
        ElectionCase::Acquired => Some(Arc::new(RecordingLeaseCache::granting())),
        ElectionCase::Reentrant => Some(Arc::new(RecordingLeaseCache::denied_with_owner(
            pod.as_ref(),
        ))),
        ElectionCase::HeldByOther => Some(Arc::new(RecordingLeaseCache::denied_with_owner(
            "another-pod",
        ))),
        ElectionCase::MissingAfterFailedAcquire => Some(Arc::new(RecordingLeaseCache::denied())),
        ElectionCase::EmptyStoredOwnerAfterFailedAcquire => {
            Some(Arc::new(RecordingLeaseCache::denied_with_owner("")))
        }
        ElectionCase::UndecodableOwnerAfterFailedAcquire => {
            Some(Arc::new(RecordingLeaseCache::denied_with_raw_owner(vec![
                0xff, 0xfe,
            ])))
        }
        ElectionCase::UnreadableAfterFailedAcquire => {
            Some(Arc::new(RecordingLeaseCache::unreadable()))
        }
    };
    let lock_manager: Option<Arc<PodLockManager<RecordingLeaseCache>>> = match case {
        ElectionCase::AbsentManager => None,
        ElectionCase::ManagerWithoutRedis => Some(Arc::new(PodLockManager::new(
            None,
            60,
            Arc::new(NoPodLockEventSink),
            pod,
        ))),
        ElectionCase::Acquired
        | ElectionCase::Reentrant
        | ElectionCase::HeldByOther
        | ElectionCase::MissingAfterFailedAcquire
        | ElectionCase::EmptyStoredOwnerAfterFailedAcquire
        | ElectionCase::UndecodableOwnerAfterFailedAcquire
        | ElectionCase::UnreadableAfterFailedAcquire => Some(Arc::new(PodLockManager::new(
            cache.clone(),
            60,
            Arc::new(NoPodLockEventSink),
            pod,
        ))),
    };
    let phases = RecordingPhases::default();
    let mut sweep = BudgetSweep {
        database: lazy_store(),
        settings: BudgetResetSettings::default(),
        lock_manager,
        phases: phases.clone(),
        tasks: TaskTracker::new(),
    };
    let (logs, guard) = captured_logs();
    let outcome = sweep.reset_budget().await;
    drop(guard);
    outcome.unwrap();

    let operations = cache
        .as_ref()
        .map_or_else(Vec::new, |lease_cache| lease_cache.operations());
    assert!(operations.iter().all(|operation| match operation {
        LeaseOperation::Set { key, .. }
        | LeaseOperation::Read { key }
        | LeaseOperation::Delete { key } => key.as_str() == LEASE_KEY,
    }));
    let expected_ttls = match case {
        ElectionCase::AbsentManager | ElectionCase::ManagerWithoutRedis => Vec::new(),
        ElectionCase::Acquired
        | ElectionCase::Reentrant
        | ElectionCase::HeldByOther
        | ElectionCase::MissingAfterFailedAcquire
        | ElectionCase::EmptyStoredOwnerAfterFailedAcquire
        | ElectionCase::UndecodableOwnerAfterFailedAcquire
        | ElectionCase::UnreadableAfterFailedAcquire => vec![900_i64],
    };
    let ttls = cache
        .as_ref()
        .map_or_else(Vec::new, |lease_cache| lease_cache.set_ttls());
    assert_eq!(ttls, expected_ttls);
    let releases = matches!(case, ElectionCase::Acquired | ElectionCase::Reentrant);
    let deletes = cache
        .as_ref()
        .map_or(0, |lease_cache| lease_cache.deletes());
    assert_eq!(deletes, usize::from(releases));

    let skip = case == ElectionCase::HeldByOther;
    let unguarded = matches!(
        case,
        ElectionCase::MissingAfterFailedAcquire
            | ElectionCase::EmptyStoredOwnerAfterFailedAcquire
            | ElectionCase::UndecodableOwnerAfterFailedAcquire
            | ElectionCase::UnreadableAfterFailedAcquire
    );
    let unreadable = matches!(
        case,
        ElectionCase::UndecodableOwnerAfterFailedAcquire
            | ElectionCase::UnreadableAfterFailedAcquire
    );
    let text = logs.text();
    assert_eq!(text.contains(SKIP_MESSAGE), skip);
    assert_eq!(text.contains(UNGUARDED_MESSAGE), unguarded);
    assert_eq!(text.contains(READ_FAILURE_MESSAGE), unreadable);
    let reads = operations
        .iter()
        .filter(|operation| matches!(operation, LeaseOperation::Read { .. }))
        .count();
    let expected_reads = match case {
        ElectionCase::AbsentManager | ElectionCase::ManagerWithoutRedis => 0,
        ElectionCase::Acquired => 1,
        ElectionCase::Reentrant
        | ElectionCase::HeldByOther
        | ElectionCase::MissingAfterFailedAcquire
        | ElectionCase::EmptyStoredOwnerAfterFailedAcquire
        | ElectionCase::UndecodableOwnerAfterFailedAcquire
        | ElectionCase::UnreadableAfterFailedAcquire => 2,
    };
    assert_eq!(reads, expected_reads);

    let calls = phases.phases();
    assert_eq!(calls.is_empty(), skip);
    assert_eq!(
        calls,
        if skip {
            Vec::new()
        } else {
            ResetPhase::ALL.to_vec()
        }
    );
}

#[tokio::test]
async fn sweep_phase_order() {
    let phases = RecordingPhases::default();
    let calls = phases.clone();
    let mut sweep: BudgetSweep<RecordingLeaseCache, _> = BudgetSweep {
        database: lazy_store(),
        settings: BudgetResetSettings::default(),
        lock_manager: None,
        phases,
        tasks: TaskTracker::new(),
    };
    sweep.reset_budget().await.unwrap();
    let expected: Vec<PhaseEvent> = ResetPhase::ALL
        .into_iter()
        .flat_map(|phase| [PhaseEvent::Start(phase), PhaseEvent::End(phase)])
        .collect();
    assert_eq!(calls.timeline(), expected);
    assert!(!calls.overlapped());
}

#[rstest]
#[case::keys(ResetPhase::Keys, "keys")]
#[case::users(ResetPhase::Users, "users")]
#[case::teams(ResetPhase::Teams, "teams")]
#[case::tiers(ResetPhase::Tiers, "tiers")]
#[case::windows(ResetPhase::Windows, "windows")]
#[tokio::test]
async fn sweep_error_short_circuit(#[case] failing: ResetPhase, #[case] label: &str) {
    let phases = RecordingPhases::failing_at(failing, "the phase refused");
    let calls = phases.clone();
    let mut sweep: BudgetSweep<RecordingLeaseCache, _> = BudgetSweep {
        database: lazy_store(),
        settings: BudgetResetSettings::default(),
        lock_manager: None,
        phases,
        tasks: TaskTracker::new(),
    };
    let error = sweep.reset_budget().await.unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("the {label} budget reset phase failed")
    );
    match error {
        SweepError::Phase { phase, source } => {
            assert_eq!(phase, failing);
            assert_eq!(source.to_string(), "the phase refused");
        }
    }
    let expected: Vec<ResetPhase> = ResetPhase::ALL
        .into_iter()
        .take_while(|phase| *phase != failing)
        .chain(std::iter::once(failing))
        .collect();
    assert_eq!(calls.phases(), expected);
    assert!(!calls.overlapped());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupCase {
    LeaderSuccess,
    LeaderFailure,
    LeaderCancel,
    Follower,
    Unguarded,
}

#[rstest]
#[case::leader_success(CleanupCase::LeaderSuccess)]
#[case::leader_failure(CleanupCase::LeaderFailure)]
#[case::leader_cancel(CleanupCase::LeaderCancel)]
#[case::follower(CleanupCase::Follower)]
#[case::unguarded(CleanupCase::Unguarded)]
#[tokio::test]
async fn sweep_leader_cleanup(#[case] case: CleanupCase) {
    let cache = Arc::new(match case {
        CleanupCase::Follower => RecordingLeaseCache::denied_with_owner("another-pod"),
        CleanupCase::Unguarded => RecordingLeaseCache::denied(),
        CleanupCase::LeaderSuccess | CleanupCase::LeaderFailure | CleanupCase::LeaderCancel => {
            RecordingLeaseCache::granting()
        }
    });
    let manager = Arc::new(PodLockManager::new(
        Some(Arc::clone(&cache)),
        60,
        Arc::new(NoPodLockEventSink),
        pod_id(),
    ));
    let started = Arc::new(Notify::new());
    let phases = match case {
        CleanupCase::LeaderFailure => {
            RecordingPhases::failing_at(ResetPhase::Teams, "the teams phase failed")
        }
        CleanupCase::LeaderCancel => {
            RecordingPhases::blocking_at(ResetPhase::Keys, Arc::clone(&started))
        }
        CleanupCase::LeaderSuccess | CleanupCase::Follower | CleanupCase::Unguarded => {
            RecordingPhases::default()
        }
    };
    let calls = phases.clone();
    let tasks = TaskTracker::new();
    let mut sweep = BudgetSweep {
        database: lazy_store(),
        settings: BudgetResetSettings::default(),
        lock_manager: Some(manager),
        phases,
        tasks: tasks.clone(),
    };
    let (outcome, deletes_on_return) = match case {
        CleanupCase::LeaderCancel => {
            let handle = tasks.spawn(async move { sweep.reset_budget().await });
            started.notified().await;
            handle.abort();
            let _cancelled = handle.await;
            tasks.close();
            tasks.wait().await;
            assert!(tasks.is_empty());
            (None, cache.deletes())
        }
        CleanupCase::LeaderSuccess
        | CleanupCase::LeaderFailure
        | CleanupCase::Follower
        | CleanupCase::Unguarded => {
            let result = sweep.reset_budget().await;
            // The source awaits release_lock in its finally block, so the delete lands before reset_budget returns.
            let deletes = cache.deletes();
            tasks.close();
            tasks.wait().await;
            (result.err(), deletes)
        }
    };
    let failure = match outcome {
        Some(Err(error)) => Some(error),
        Some(Ok(())) | None => None,
    };
    assert_eq!(failure.is_some(), case == CleanupCase::LeaderFailure);
    if let Some(error) = failure {
        match error {
            SweepError::Phase { phase, source } => {
                assert_eq!(phase, ResetPhase::Teams);
                assert_eq!(source.to_string(), "the teams phase failed");
            }
        }
    }
    let releases = matches!(
        case,
        CleanupCase::LeaderSuccess | CleanupCase::LeaderFailure | CleanupCase::LeaderCancel
    );
    assert_eq!(deletes_on_return, usize::from(releases));
    assert_eq!(cache.deletes(), usize::from(releases));
    let expected_phases: Vec<ResetPhase> = match case {
        CleanupCase::LeaderSuccess | CleanupCase::Unguarded => ResetPhase::ALL.to_vec(),
        CleanupCase::LeaderFailure => ResetPhase::ALL.into_iter().take(3).collect(),
        CleanupCase::LeaderCancel => ResetPhase::ALL.into_iter().take(1).collect(),
        CleanupCase::Follower => Vec::new(),
    };
    assert_eq!(calls.phases(), expected_phases);
}
