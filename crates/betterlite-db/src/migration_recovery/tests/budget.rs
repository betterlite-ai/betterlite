use std::time::Duration;

use rstest::rstest;
use tracing::subscriber::set_default;

use crate::migration_ledger::budget::{BudgetError, MigrateAttemptBudget};
use crate::migration_recovery::RecoveryError;
use crate::migration_recovery::outcome::DeployOutcome;
use crate::migration_recovery::policy::{PassSettings, run_database_v2, run_passes};
use crate::migration_recovery::tests::support::{
    FakeJitter, RecordingPause, ScriptedSteps, capture_logs,
};

const DEPLOY_TIMEOUT: Duration = Duration::from_secs(42);

fn contention() -> DeployOutcome {
    DeployOutcome::Contention {
        detail: "another process held the migration lock".to_owned(),
    }
}

#[tokio::test(start_paused = true)]
async fn timeout_warning_and_attempt_spend() {
    let steps = ScriptedSteps::default();
    (0_u32..4).for_each(|_| steps.deploying(DeployOutcome::TimedOut));
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let error = run_database_v2(
        &steps,
        MigrateAttemptBudget::default(),
        DEPLOY_TIMEOUT,
        &jitter,
        &pause,
    )
    .await
    .expect_err("four timeouts exhaust the failed attempts");
    drop(guard);
    let captured = logs.contents();
    assert!(
        captured.contains("attempt 1 timed out after 42s"),
        "{captured}"
    );
    assert!(
        captured.contains("attempt 4 timed out after 42s"),
        "{captured}"
    );
    assert!(
        captured.contains("LITELLM_PRISMA_MIGRATE_DEPLOY_TIMEOUT"),
        "{captured}"
    );
    assert!(matches!(error, RecoveryError::AttemptsExhausted { .. }));
}

#[rstest]
#[case::lowest(0, 5)]
#[case::one_second(1_000_000_000, 6)]
#[case::fraction_is_floored(1_999_999_999, 6)]
#[case::highest(9_999_999_999, 14)]
#[tokio::test(start_paused = true)]
async fn integer_jitter_five_to_fourteen(#[case] draw: u64, #[case] expected: u64) {
    let steps = ScriptedSteps::default();
    steps.deploying(DeployOutcome::TimedOut);
    steps.deploying(DeployOutcome::Complete);
    let jitter = FakeJitter(draw);
    let pause = RecordingPause::default();
    run_database_v2(
        &steps,
        MigrateAttemptBudget::default(),
        DEPLOY_TIMEOUT,
        &jitter,
        &pause,
    )
    .await
    .expect("the retry succeeds");
    assert_eq!(pause.delays(), vec![Duration::from_secs(expected)]);
}

#[tokio::test(start_paused = true)]
async fn exhausted_attempt_still_sleeps() {
    let steps = ScriptedSteps::default();
    (0_u32..4).for_each(|_| steps.deploying(DeployOutcome::TimedOut));
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    run_database_v2(
        &steps,
        MigrateAttemptBudget::default(),
        DEPLOY_TIMEOUT,
        &jitter,
        &pause,
    )
    .await
    .expect_err("four timeouts exhaust the failed attempts");
    assert_eq!(pause.delays().len(), 4);
}

#[tokio::test(start_paused = true)]
async fn contention_only_elapsed_budget() {
    let steps = ScriptedSteps::default();
    steps.deploying_after(contention(), Duration::from_secs(100));
    steps.deploying_after(contention(), Duration::from_secs(200));
    steps.deploying(DeployOutcome::Complete);
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    run_database_v2(
        &steps,
        MigrateAttemptBudget::default(),
        DEPLOY_TIMEOUT,
        &jitter,
        &pause,
    )
    .await
    .expect("contention never spends a failure attempt");
    drop(guard);
    assert!(pause.delays().is_empty());
    assert_eq!(steps.deploy_timeouts().len(), 3);
    assert!(
        logs.contents().contains("does not spend a failure attempt"),
        "{}",
        logs.contents()
    );
}

#[tokio::test(start_paused = true)]
async fn contention_exhaustion() {
    let steps = ScriptedSteps::default();
    steps.deploying_after(contention(), Duration::from_secs(100));
    steps.deploying_after(contention(), Duration::from_secs(60));
    let budget = MigrateAttemptBudget {
        attempts_left: 4,
        contention_left: Duration::from_secs(150),
    };
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    let error = run_database_v2(&steps, budget, DEPLOY_TIMEOUT, &jitter, &pause)
        .await
        .expect_err("the contention budget runs out");
    assert!(matches!(
        error,
        RecoveryError::Budget(BudgetError::ContentionExhausted)
    ));
    assert!(
        error
            .to_string()
            .contains("Timed out waiting for Prisma's migration advisory lock"),
        "{error}"
    );
    assert!(pause.delays().is_empty());
    assert_eq!(steps.deploy_timeouts().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn shared_timeout_policy_consumed() {
    let steps = ScriptedSteps::default();
    steps.deploying_after(contention(), Duration::from_secs(76));
    steps.deploying_after(contention(), Duration::from_secs(2));
    let settings = PassSettings {
        deploy_timeout: DEPLOY_TIMEOUT,
        lock_timeout: Duration::from_secs(77),
    };
    let jitter = FakeJitter(0);
    let pause = RecordingPause::default();
    let error = run_passes(&steps, settings, &jitter, &pause)
        .await
        .expect_err("the configured lock timeout bounds contention");
    assert!(matches!(
        error,
        RecoveryError::Budget(BudgetError::ContentionExhausted)
    ));
    assert_eq!(
        steps.deploy_timeouts(),
        vec![DEPLOY_TIMEOUT, DEPLOY_TIMEOUT],
        "every deploy attempt receives the configured deploy timeout"
    );
    assert!(pause.delays().is_empty());
}
