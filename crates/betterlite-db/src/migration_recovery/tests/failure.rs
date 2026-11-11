use std::sync::Arc;
use std::time::Duration;

use tokio::task::yield_now;

use rstest::rstest;
use tokio::task::JoinSet;

use crate::migration_ledger::budget::MigrateAttemptBudget;
use crate::migration_ledger::ledger_reads::{FailedLookup, FailedMigration};
use crate::migration_recovery::RecoveryError;
use crate::migration_recovery::messages;
use crate::migration_recovery::outcome::{
    DeployOutcome, FailedIdentity, FailedStep, is_permission_error,
};
use crate::migration_recovery::policy::{PassSettings, run_database_v2, run_passes};
use crate::migration_recovery::tests::support::{FakeJitter, RecordingPause, ScriptedSteps};

const DEPLOY_TIMEOUT: Duration = Duration::from_secs(42);

fn settings() -> PassSettings {
    PassSettings {
        deploy_timeout: DEPLOY_TIMEOUT,
        lock_timeout: Duration::from_secs(600),
    }
}

fn identity() -> FailedIdentity {
    FailedIdentity {
        name: "20240101000000_a".to_owned(),
        started_at: "2024-01-01 00:00:00+00".to_owned(),
    }
}

fn found(logs: &str, recovered: bool) -> FailedLookup {
    FailedLookup::Found(FailedMigration {
        logs: logs.to_owned(),
        recovered,
    })
}

#[tokio::test(start_paused = true)]
async fn unfinished_history_exact_identity() {
    let steps = ScriptedSteps::default();
    let reported = identity();
    steps.deploying(DeployOutcome::UnfinishedHistory {
        identity: Some(reported.clone()),
    });
    steps.history(
        &FailedIdentity {
            name: reported.name.clone(),
            started_at: "2020-01-01 00:00:00+00".to_owned(),
        },
        found("", true),
    );
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    let error = run_database_v2(
        &steps,
        MigrateAttemptBudget::default(),
        DEPLOY_TIMEOUT,
        &jitter,
        &pause,
    )
    .await
    .expect_err("a row with another start time is not this migration's evidence");
    assert!(matches!(error, RecoveryError::UnverifiedCompletion { .. }));
    let message = error.to_string();
    assert!(message.contains(&reported.name), "{message}");
    assert!(message.contains(&reported.started_at), "{message}");
    assert_eq!(
        steps.events(),
        vec![
            "ahead".to_owned(),
            "deploy".to_owned(),
            format!("recover:{}", reported.name),
            format!("history:{}@{}", reported.name, reported.started_at),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn ledger_deadlock_rolls_back_before_retry() {
    let steps = ScriptedSteps::default();
    let reported = identity();
    steps.deploying(DeployOutcome::UnfinishedHistory {
        identity: Some(reported.clone()),
    });
    steps.history(&reported, found("ERROR: deadlock detected", false));
    steps.deploying(DeployOutcome::Complete);
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    run_passes(&steps, settings(), &jitter, &pause)
        .await
        .expect("the deadlock rollback lets the next pass deploy");
    assert_eq!(
        steps.events(),
        vec![
            "ahead".to_owned(),
            "deploy".to_owned(),
            format!("recover:{}", reported.name),
            format!("history:{}@{}", reported.name, reported.started_at),
            format!("rollback:{}", reported.name),
            "ahead".to_owned(),
            "deploy".to_owned(),
        ]
    );
    assert_eq!(pause.delays(), vec![Duration::from_secs(5)]);
}

#[tokio::test(start_paused = true)]
async fn concurrent_recovered_row_spends_attempt() {
    let steps = ScriptedSteps::default();
    let reported = identity();
    steps.deploying(DeployOutcome::UnfinishedHistory {
        identity: Some(reported.clone()),
    });
    steps.history(&reported, found("", true));
    steps.deploying(DeployOutcome::Complete);
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    run_passes(&steps, settings(), &jitter, &pause)
        .await
        .expect("the concurrent recovery lets the next attempt deploy");
    assert!(
        !steps
            .events()
            .iter()
            .any(|event| event.starts_with("rollback:"))
    );
    assert_eq!(pause.delays(), vec![Duration::from_secs(5)]);
    assert_eq!(steps.deploy_timeouts().len(), 2);
}

fn assert_recovery_guidance(message: &str) {
    assert!(
        message.starts_with(
            "Migration completion could not be verified. LiteLLM startup has stopped."
        ),
        "{message}"
    );
    assert!(
        message.contains("stop other migration runners and inspect _prisma_migrations"),
        "{message}"
    );
    assert!(
        message.contains("Only after verifying every migration change is present"),
        "{message}"
    );
    assert!(
        message.contains("prisma migrate resolve --applied <migration_name>, then retry startup"),
        "{message}"
    );
    assert!(
        message.contains("Only after verifying no migration changes remain"),
        "{message}"
    );
    assert!(
        message
            .contains("prisma migrate resolve --rolled-back <migration_name>, then retry startup"),
        "{message}"
    );
    assert!(
        message.contains("This command updates history; it does not undo SQL."),
        "{message}"
    );
    assert!(
        message.contains("Repeated restarts alone will not resolve this state."),
        "{message}"
    );
}

#[test]
fn uncertain_completion_full_guidance() {
    let reported = identity();
    let message = messages::unverified_completion(Some(&reported), "");
    assert_recovery_guidance(&message);
    assert!(
        message.contains("20240101000000_a started at 2024-01-01 00:00:00+00"),
        "{message}"
    );
    let missing = messages::unverified_completion(None, "the migration history could not be read");
    assert!(
        missing.contains("the migration history could not be read"),
        "{missing}"
    );
}

#[rstest]
#[case::sqlstate_42501(Some("42501"), "the step failed", true)]
#[case::database_error_code(None, "Database error code: 42501", true)]
#[case::must_own_table(None, "MUST BE OWNER OF TABLE orders", true)]
#[case::denied_schema(None, "permission denied for schema billing", true)]
#[case::denied_table(None, "permission denied for table users", true)]
#[case::must_own_schema(None, "must be owner of schema public", true)]
#[case::syntax_error(Some("42601"), "syntax error at or near FROM", false)]
#[case::deadlock(None, "deadlock detected", false)]
fn permission_patterns_and_sqlstate(
    #[case] sqlstate: Option<&str>,
    #[case] detail: &str,
    #[case] expected: bool,
) {
    assert_eq!(is_permission_error(sqlstate, detail), expected);
}

#[tokio::test(start_paused = true)]
async fn permission_before_deadlock() {
    let steps = ScriptedSteps::default();
    steps.deploying(DeployOutcome::FailedStep(FailedStep {
        name: Some("20240101000000_a".to_owned()),
        sqlstate: Some("42501".to_owned()),
        detail: "permission denied for table users; deadlock detected".to_owned(),
    }));
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    let error = run_database_v2(
        &steps,
        MigrateAttemptBudget::default(),
        DEPLOY_TIMEOUT,
        &jitter,
        &pause,
    )
    .await
    .expect_err("the permission rejection outranks the deadlock retry");
    assert!(matches!(
        error,
        RecoveryError::InsufficientPrivileges { .. }
    ));
    assert!(
        !steps
            .events()
            .iter()
            .any(|event| event.starts_with("rollback:"))
    );
    assert!(pause.delays().is_empty());
}

#[tokio::test(start_paused = true)]
async fn named_step_deadlock() {
    let steps = ScriptedSteps::default();
    steps.deploying(DeployOutcome::FailedStep(FailedStep {
        name: Some("20240101000000_a".to_owned()),
        sqlstate: Some("40P01".to_owned()),
        detail: "deadlock detected".to_owned(),
    }));
    steps.deploying(DeployOutcome::Complete);
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    run_passes(&steps, settings(), &jitter, &pause)
        .await
        .expect("the named deadlock rolls its history back and retries");
    assert!(
        steps
            .events()
            .contains(&"rollback:20240101000000_a".to_owned())
    );
    assert_eq!(pause.delays(), vec![Duration::from_secs(5)]);
}

#[tokio::test(start_paused = true)]
async fn bare_deadlock_before_contention() {
    let steps = ScriptedSteps::default();
    steps.deploying_after(
        DeployOutcome::Contention {
            detail: "deadlock detected while waiting for the migration lock".to_owned(),
        },
        Duration::from_secs(5),
    );
    steps.deploying(DeployOutcome::Complete);
    let budget = MigrateAttemptBudget {
        attempts_left: 4,
        contention_left: Duration::from_secs(1),
    };
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    run_database_v2(&steps, budget, DEPLOY_TIMEOUT, &jitter, &pause)
        .await
        .expect("a deadlock marker spends an attempt instead of the contention budget");
    assert_eq!(pause.delays(), vec![Duration::from_secs(5)]);
}

#[tokio::test(start_paused = true)]
async fn unknown_failure_manual_intervention() {
    let steps = ScriptedSteps::default();
    steps.deploying(DeployOutcome::Failed {
        detail: "the database returned an unexpected error".to_owned(),
    });
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    let error = run_database_v2(
        &steps,
        MigrateAttemptBudget::default(),
        DEPLOY_TIMEOUT,
        &jitter,
        &pause,
    )
    .await
    .expect_err("an unrecognized failure stops startup");
    assert!(matches!(error, RecoveryError::ManualIntervention { .. }));
    let message = error.to_string();
    assert!(
        message.contains(
            "Database migration failed and cannot be auto-recovered. Manual intervention required."
        ),
        "{message}"
    );
    assert!(message.contains("unexpected error"), "{message}");
}

#[tokio::test(start_paused = true)]
async fn exhaustion_full_message() {
    let steps = ScriptedSteps::default();
    (0_u32..4).for_each(|_| {
        steps.deploying(DeployOutcome::Deadlock {
            detail: "deadlock detected".to_owned(),
        });
    });
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    let error = run_database_v2(
        &steps,
        MigrateAttemptBudget::default(),
        DEPLOY_TIMEOUT,
        &jitter,
        &pause,
    )
    .await
    .expect_err("four deadlocks exhaust the failed attempts");
    let message = error.to_string();
    assert!(
        message.contains("after 4 attempts that made no progress (timeouts or deadlock retries)"),
        "{message}"
    );
    assert!(message.contains("connectivity"), "{message}");
    assert!(message.contains("_prisma_migrations"), "{message}");
    assert!(
        message.contains("LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT"),
        "{message}"
    );
    assert_eq!(pause.delays().len(), 4);
}

#[test]
fn native_diagnostics_carry_sqlstate_and_ledger_identity() {
    let step = FailedStep {
        name: Some("20240101000000_a".to_owned()),
        sqlstate: Some("42501".to_owned()),
        detail: "permission denied for table users".to_owned(),
    };
    let permission = messages::insufficient_privileges(&step);
    assert!(
        permission.contains(
            "Database migration failed due to insufficient permissions. Please grant the required privileges and retry."
        ),
        "{permission}"
    );
    assert!(
        permission.contains("Migration: 20240101000000_a"),
        "{permission}"
    );
    assert!(permission.contains("SQLSTATE: 42501"), "{permission}");
    assert!(
        permission.contains("permission denied for table users"),
        "{permission}"
    );
    let manual = messages::manual_for_step(&step);
    assert!(manual.contains("SQLSTATE: 42501"), "{manual}");
    assert!(manual.contains("Migration: 20240101000000_a"), "{manual}");
    let reported = identity();
    let unverified = messages::unverified_completion(Some(&reported), "");
    assert!(
        unverified.contains("20240101000000_a started at 2024-01-01 00:00:00+00"),
        "{unverified}"
    );
}

#[tokio::test(start_paused = true)]
async fn rollback_cancel_propagates() {
    let steps = Arc::new(ScriptedSteps::default());
    let reported = identity();
    steps.deploying(DeployOutcome::UnfinishedHistory {
        identity: Some(reported.clone()),
    });
    steps.history(&reported, found("deadlock detected", false));
    steps.block_rollback();
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    let running = Arc::clone(&steps);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        run_database_v2(
            running.as_ref(),
            MigrateAttemptBudget::default(),
            DEPLOY_TIMEOUT,
            &jitter,
            &pause,
        )
        .await
    });
    yield_now().await;
    yield_now().await;
    yield_now().await;
    tasks.abort_all();
    let outcome = tasks
        .join_next()
        .await
        .expect("the pass task runs")
        .expect_err("cancellation propagates out of the pass");
    assert!(outcome.is_cancelled());
    assert!(
        steps
            .events()
            .iter()
            .any(|event| event.starts_with("rollback:"))
    );
}
