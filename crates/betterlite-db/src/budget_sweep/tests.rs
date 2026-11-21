#![cfg(test)]

mod support;
mod sweep_assembly;

use std::error::Error as _;
use std::iter::once;
use std::sync::Arc;
use std::time::Duration;

use rstest::rstest;
use tokio::sync::Notify;
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use crate::budget_sweep::BudgetSweep;
use crate::budget_sweep::error::SweepError;
use crate::budget_sweep::phase::ResetPhase;
use crate::budget_sweep::settings::BudgetResetSettings;
use crate::queues::pod_lock::NoPodLockEventSink;
use crate::queues::pod_lock::PodLockManager;
use crate::queues::pod_lock::owner::PodId;

use self::support::{
    LeaseOperation, PhaseEvent, PhaseFault, RecordingLeaseCache, RecordingPhases, captured_logs,
    lazy_store, pod_id,
};

const LEASE_KEY: &str = "cronjob_lock:reset_budget_job";
const SKIP_MESSAGE: &str = "another pod holds the reset lease, skipping this tick";
const UNGUARDED_MESSAGE: &str = "could not take the reset lease and no other pod holds it";
const READ_FAILURE_MESSAGE: &str = "could not read the reset lease";

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

fn election_cache(case: ElectionCase, pod: &str) -> Option<Arc<RecordingLeaseCache>> {
    match case {
        ElectionCase::AbsentManager | ElectionCase::ManagerWithoutRedis => None,
        ElectionCase::Acquired => Some(Arc::new(RecordingLeaseCache::granting())),
        ElectionCase::Reentrant => Some(Arc::new(RecordingLeaseCache::denied_with_owner(pod))),
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
    }
}

fn election_lock_manager(
    case: ElectionCase,
    cache: Option<Arc<RecordingLeaseCache>>,
    pod: PodId,
) -> Option<Arc<PodLockManager<RecordingLeaseCache>>> {
    if case == ElectionCase::AbsentManager {
        return None;
    }
    Some(Arc::new(PodLockManager::new(
        cache,
        60,
        Arc::new(NoPodLockEventSink),
        pod,
    )))
}

fn assert_election_operations(case: ElectionCase, operations: &[LeaseOperation]) {
    assert!(operations.iter().all(|operation| match operation {
        LeaseOperation::Set(key, _) | LeaseOperation::Read(key) | LeaseOperation::Delete(key) => {
            key.as_str() == LEASE_KEY
        }
    }));
    // The source passes lock_ttl=900 explicitly, and reaches no Redis at all without a manager or a cache.
    let expected_ttls = if matches!(
        case,
        ElectionCase::AbsentManager | ElectionCase::ManagerWithoutRedis
    ) {
        Vec::new()
    } else {
        vec![900_i64]
    };
    let ttls: Vec<i64> = operations
        .iter()
        .filter_map(|operation| match operation {
            LeaseOperation::Set(_, ttl) => Some(*ttl),
            LeaseOperation::Read(_) | LeaseOperation::Delete(_) => None,
        })
        .collect();
    assert_eq!(ttls, expected_ttls);
    let released = matches!(case, ElectionCase::Acquired | ElectionCase::Reentrant);
    let delete_count = operations
        .iter()
        .filter(|operation| matches!(operation, LeaseOperation::Delete(_)))
        .count();
    assert_eq!(delete_count, usize::from(released));
}

fn assert_election_logs(case: ElectionCase, operations: &[LeaseOperation], text: &str) {
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
    assert_eq!(text.contains(SKIP_MESSAGE), skip);
    assert_eq!(text.contains(UNGUARDED_MESSAGE), unguarded);
    assert_eq!(text.contains(READ_FAILURE_MESSAGE), unreadable);
    let reads = operations
        .iter()
        .filter(|operation| matches!(operation, LeaseOperation::Read(_)))
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
    let cache = election_cache(case, pod.as_ref());
    let lease_cache = cache.as_deref();
    let phases = RecordingPhases::default();
    let mut sweep = BudgetSweep {
        database: lazy_store(),
        settings: BudgetResetSettings::default(),
        lock_manager: election_lock_manager(case, cache.clone(), pod),
        phases: phases.clone(),
        tasks: TaskTracker::new(),
    };
    let (logs, guard) = captured_logs();
    let outcome = sweep.reset_budget().await;
    drop(guard);
    outcome.unwrap();
    let operations = lease_cache.map_or_else(Vec::new, RecordingLeaseCache::operations);
    assert_election_operations(case, &operations);
    assert_election_logs(case, &operations, &logs.text());
    let calls = phases.phases();
    let skip = case == ElectionCase::HeldByOther;
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
    let source = error.source().unwrap();
    assert_eq!(source.to_string(), "the phase refused");
    let expected: Vec<ResetPhase> = ResetPhase::ALL
        .into_iter()
        .take_while(|phase| *phase != failing)
        .chain(once(failing))
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

fn cleanup_cache(case: CleanupCase) -> Arc<RecordingLeaseCache> {
    Arc::new(match case {
        CleanupCase::Follower => RecordingLeaseCache::denied_with_owner("another-pod"),
        CleanupCase::Unguarded => RecordingLeaseCache::denied(),
        CleanupCase::LeaderSuccess | CleanupCase::LeaderFailure | CleanupCase::LeaderCancel => {
            RecordingLeaseCache::granting()
        }
    })
}

fn cleanup_phases(case: CleanupCase, started: &Arc<Notify>) -> RecordingPhases {
    match case {
        CleanupCase::LeaderFailure => {
            RecordingPhases::failing_at(ResetPhase::Teams, "the teams phase failed")
        }
        CleanupCase::LeaderCancel => {
            RecordingPhases::blocking_at(ResetPhase::Keys, Arc::clone(started))
        }
        CleanupCase::LeaderSuccess | CleanupCase::Follower | CleanupCase::Unguarded => {
            RecordingPhases::default()
        }
    }
}

fn deletes(cache: &RecordingLeaseCache) -> usize {
    cache
        .operations()
        .iter()
        .filter(|operation| matches!(operation, LeaseOperation::Delete(_)))
        .count()
}

async fn run_cleanup(
    case: CleanupCase,
    mut sweep: BudgetSweep<RecordingLeaseCache, RecordingPhases>,
    tasks: TaskTracker,
    started: &Arc<Notify>,
    cache: &RecordingLeaseCache,
) -> (Option<SweepError<PhaseFault>>, usize) {
    let result = match case {
        CleanupCase::LeaderCancel => {
            let handle = tasks.spawn(async move { sweep.reset_budget().await });
            started.notified().await;
            handle.abort();
            let _cancelled = handle.await;
            tasks.close();
            timeout(Duration::from_secs(1), tasks.wait()).await.unwrap();
            assert!(tasks.is_empty());
            return (None, deletes(cache));
        }
        CleanupCase::LeaderSuccess
        | CleanupCase::LeaderFailure
        | CleanupCase::Follower
        | CleanupCase::Unguarded => sweep.reset_budget().await,
    };
    // The source awaits release_lock in its finally block, so the delete lands before reset_budget returns.
    let released = deletes(cache);
    tasks.close();
    tasks.wait().await;
    (result.err(), released)
}

fn assert_cleanup(
    case: CleanupCase,
    failure: Option<SweepError<PhaseFault>>,
    deletes_on_return: usize,
    cache: &RecordingLeaseCache,
    calls: &RecordingPhases,
) {
    assert_eq!(failure.is_some(), case == CleanupCase::LeaderFailure);
    if let Some(error) = failure {
        assert_eq!(error.to_string(), "the teams budget reset phase failed");
        assert_eq!(
            error.source().unwrap().to_string(),
            "the teams phase failed"
        );
    }
    let released = matches!(
        case,
        CleanupCase::LeaderSuccess | CleanupCase::LeaderFailure | CleanupCase::LeaderCancel
    );
    assert_eq!(deletes_on_return, usize::from(released));
    assert_eq!(deletes(cache), usize::from(released));
    let expected_phases: Vec<ResetPhase> = match case {
        CleanupCase::LeaderSuccess | CleanupCase::Unguarded => ResetPhase::ALL.to_vec(),
        CleanupCase::LeaderFailure => ResetPhase::ALL.into_iter().take(3).collect(),
        CleanupCase::LeaderCancel => ResetPhase::ALL.into_iter().take(1).collect(),
        CleanupCase::Follower => Vec::new(),
    };
    assert_eq!(calls.phases(), expected_phases);
}

#[rstest]
#[case::leader_success(CleanupCase::LeaderSuccess)]
#[case::leader_failure(CleanupCase::LeaderFailure)]
#[case::leader_cancel(CleanupCase::LeaderCancel)]
#[case::follower(CleanupCase::Follower)]
#[case::unguarded(CleanupCase::Unguarded)]
#[tokio::test]
async fn sweep_leader_cleanup(#[case] case: CleanupCase) {
    let cache = cleanup_cache(case);
    let manager = Arc::new(PodLockManager::new(
        Some(Arc::clone(&cache)),
        60,
        Arc::new(NoPodLockEventSink),
        pod_id(),
    ));
    let started = Arc::new(Notify::new());
    let phases = cleanup_phases(case, &started);
    let calls = phases.clone();
    let tasks = TaskTracker::new();
    let sweep = BudgetSweep {
        database: lazy_store(),
        settings: BudgetResetSettings::default(),
        lock_manager: Some(manager),
        phases,
        tasks: tasks.clone(),
    };
    let (failure, deletes_on_return) = run_cleanup(case, sweep, tasks, &started, &cache).await;
    assert_cleanup(case, failure, deletes_on_return, &cache, &calls);
}
