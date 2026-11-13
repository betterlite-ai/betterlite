#![cfg(test)]

mod fakes;
mod postgres;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use rstest::rstest;
use secrecy::SecretString;
use tokio::sync::Notify;
use tracing_subscriber::layer::SubscriberExt as _;

use super::engine::EngineState;
use super::lock::TokioLock;
use super::policy::{AuthReconnect, ReconnectHistory, ReconnectRequest};
use super::settings::ReconnectSettings;
use super::stale::StaleReadEngine;

use fakes::{
    CapturedLogs, Effect, EngineScript, FakeEngine, FakeLock, LockMode, ProbeOutcome,
    RecreateOutcome, ScriptedClock, StepOutcome,
};

const URL: &str = "postgresql://user:pw@db.example:5432/litellm";

type Policy = AuthReconnect<FakeEngine, ScriptedClock, FakeLock>;

fn settings() -> ReconnectSettings {
    ReconnectSettings {
        cooldown: Duration::from_secs(15),
        escalation_threshold: 3,
        watchdog_timeout: Duration::from_secs(30),
        auth_timeout: Duration::from_secs(2),
        auth_lock_timeout: Duration::from_millis(100),
    }
}

fn policy_with_engine(engine: FakeEngine, effects: fakes::Effects) -> Policy {
    AuthReconnect::new(
        engine,
        settings(),
        ScriptedClock::from_seconds(&[1000.0]),
        FakeLock::new(LockMode::Free, effects),
        ReconnectHistory::default(),
    )
}

async fn seed_failure(policy: &Policy, url: &SecretString, reason: &str) {
    let seeding = request(reason, url, true, false);
    assert!(!policy.attempt_db_reconnect(&seeding, None).await);
}

fn request<'request>(
    reason: &'request str,
    url: &'request SecretString,
    force: bool,
    force_recreate: bool,
) -> ReconnectRequest<'request> {
    ReconnectRequest {
        reason,
        force,
        force_recreate,
        timeout: None,
        lock_timeout: None,
        database_url: Some(url),
    }
}

fn logged(logs: &CapturedLogs, level: tracing::Level) -> Vec<BTreeMap<String, String>> {
    logs.entries()
        .into_iter()
        .filter(|entry| entry.level == level)
        .map(|entry| entry.fields)
        .collect()
}

fn answers(fields: &BTreeMap<String, String>, name: &str, value: &str) -> bool {
    fields.get(name).is_some_and(|entry| entry == value)
}

#[tokio::test]
async fn equal_generation_different_handle() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.routing = true;
    script.probes.push_back(ProbeOutcome::Value(false));
    script.probes.push_back(ProbeOutcome::Value(false));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine.clone(), Arc::clone(&effects));
    let url = SecretString::from(URL);
    let live = StaleReadEngine::observe(&engine.reader_observation()).expect("the reader is named");
    let first = request("first", &url, false, false);
    assert!(policy.attempt_db_reconnect(&first, Some(&live)).await);
    // the reader is still live, so its own repair waives the cooldown it just stamped
    let waived = request("live", &url, false, false);
    assert!(policy.attempt_db_reconnect(&waived, Some(&live)).await);
    // the writer carries the same generation number, so only the handle says it was replaced
    let switched =
        StaleReadEngine::observe(&engine.writer_observation()).expect("the writer is named");
    let attempts = fakes::recorded(&effects).len();
    let gated = request("switched", &url, false, false);
    assert!(!policy.attempt_db_reconnect(&gated, Some(&switched)).await);
    assert_eq!(fakes::recorded(&effects).len(), attempts);
}

#[tokio::test]
async fn stale_live_waives_once() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.routing = true;
    script.probes.push_back(ProbeOutcome::Value(false));
    script.probes.push_back(ProbeOutcome::Value(true));
    script
        .recreates
        .push_back(RecreateOutcome::Failure("recreate failed".to_owned()));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine.clone(), Arc::clone(&effects));
    let url = SecretString::from(URL);
    let stale =
        StaleReadEngine::observe(&engine.reader_observation()).expect("the reader is named");
    let first = request("first_attempt", &url, false, false);
    assert!(policy.attempt_db_reconnect(&first, Some(&stale)).await);
    // the live engine has no failed repair yet, so the cooldown does not gate its own repair
    let waived = request("repair_fails", &url, false, false);
    assert!(!policy.attempt_db_reconnect(&waived, Some(&stale)).await);
    assert!(policy.has_failed_repair_for(&engine.reader_handle(), 0));
    // the failed repair of this engine now makes the same live observation respect the cooldown
    let attempts = fakes::recorded(&effects).len();
    let recorded = request("recorded_engine", &url, false, false);
    assert!(!policy.attempt_db_reconnect(&recorded, Some(&stale)).await);
    assert_eq!(fakes::recorded(&effects).len(), attempts);
    let forced = request("forced", &url, true, false);
    assert!(policy.attempt_db_reconnect(&forced, Some(&stale)).await);
}

#[tokio::test]
async fn failed_repair_cooldown() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.probes.push_back(ProbeOutcome::Value(true));
    script
        .recreates
        .push_back(RecreateOutcome::Failure("recreate failed".to_owned()));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine.clone(), Arc::clone(&effects));
    let url = SecretString::from(URL);
    let stale =
        StaleReadEngine::observe(&engine.writer_observation()).expect("the writer is named");
    let repair = request("forced_repair", &url, true, false);
    assert!(!policy.attempt_db_reconnect(&repair, Some(&stale)).await);
    let attempts = fakes::recorded(&effects).len();
    let cooldown = request("cooldown", &url, false, false);
    assert!(!policy.attempt_db_reconnect(&cooldown, Some(&stale)).await);
    assert_eq!(fakes::recorded(&effects).len(), attempts);
}

#[tokio::test]
async fn force_bypasses_both_checks() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.probes.push_back(ProbeOutcome::Value(true));
    script
        .recreates
        .push_back(RecreateOutcome::Failure("recreate failed".to_owned()));
    script.probes.push_back(ProbeOutcome::Value(false));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine.clone(), Arc::clone(&effects));
    let url = SecretString::from(URL);
    let stale =
        StaleReadEngine::observe(&engine.writer_observation()).expect("the writer is named");
    let first = request("first", &url, false, false);
    assert!(!policy.attempt_db_reconnect(&first, Some(&stale)).await);
    let attempts = fakes::recorded(&effects).len();
    let cooldown = request("cooldown", &url, false, false);
    assert!(!policy.attempt_db_reconnect(&cooldown, Some(&stale)).await);
    assert_eq!(fakes::recorded(&effects).len(), attempts);
    let forced = request("forced", &url, true, false);
    assert!(policy.attempt_db_reconnect(&forced, Some(&stale)).await);
    assert!(fakes::recorded(&effects).len() > attempts);
}

#[tokio::test]
async fn queued_caller_rechecks() {
    let effects = fakes::effects();
    let gate = Arc::new(Notify::new());
    let mut script = EngineScript::default();
    script.probes.push_back(ProbeOutcome::Value(true));
    script.probes.push_back(ProbeOutcome::Value(true));
    script.recreates.push_back(RecreateOutcome::Value(true));
    script.hold_recreate = Some(Arc::clone(&gate));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = Arc::new(AuthReconnect::new(
        engine,
        settings(),
        ScriptedClock::from_seconds(&[1000.0]),
        TokioLock::default(),
        ReconnectHistory::default(),
    ));
    let url = SecretString::from(URL);
    let first = {
        let policy = Arc::clone(&policy);
        let url = url.clone();
        tokio::spawn(async move {
            policy
                .attempt_db_reconnect(&request("first", &url, false, false), None)
                .await
        })
    };
    while !fakes::recorded(&effects)
        .iter()
        .any(|effect| matches!(effect, Effect::Recreate { .. }))
    {
        tokio::task::yield_now().await;
    }
    let second = {
        let policy = Arc::clone(&policy);
        let url = url.clone();
        tokio::spawn(async move {
            policy
                .attempt_db_reconnect(&request("queued", &url, false, false), None)
                .await
        })
    };
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    gate.notify_waiters();
    assert!(first.await.expect("the first caller finishes"));
    assert!(!second.await.expect("the queued caller finishes"));
    let recreates = fakes::recorded(&effects)
        .into_iter()
        .filter(|effect| matches!(effect, Effect::Recreate { .. }))
        .count();
    assert_eq!(recreates, 1);
}

#[tokio::test]
async fn completion_time_cooldown() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.probes.push_back(ProbeOutcome::Value(false));
    script.probes.push_back(ProbeOutcome::Value(false));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = AuthReconnect::new(
        engine,
        ReconnectSettings {
            cooldown: Duration::from_secs(5),
            escalation_threshold: 3,
            watchdog_timeout: Duration::from_secs(30),
            auth_timeout: Duration::from_secs(2),
            auth_lock_timeout: Duration::from_millis(100),
        },
        ScriptedClock::from_seconds(&[0.0, 0.0, 10.0, 14.0, 15.0, 15.0, 15.0]),
        FakeLock::new(LockMode::Free, Arc::clone(&effects)),
        ReconnectHistory::default(),
    );
    let url = SecretString::from(URL);
    let first = request("first", &url, false, false);
    assert!(policy.attempt_db_reconnect(&first, None).await);
    // a stamp at entry (0s) would let this one through; the completion stamp (10s) does not
    let within = request("within_window", &url, false, false);
    assert!(!policy.attempt_db_reconnect(&within, None).await);
    let elapsed = request("window_elapsed", &url, false, false);
    assert!(policy.attempt_db_reconnect(&elapsed, None).await);
}

#[tokio::test]
async fn lock_timeout_and_acquire_error() {
    let logs = CapturedLogs::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
    let url = SecretString::from(URL);
    let effects = fakes::effects();
    let engine = FakeEngine::new(EngineScript::default(), Arc::clone(&effects));
    let waiting = AuthReconnect::new(
        engine,
        settings(),
        ScriptedClock::from_seconds(&[1000.0]),
        FakeLock::new(LockMode::Timeout, Arc::clone(&effects)),
        ReconnectHistory::default(),
    );
    let timeout = request("lock_timeout", &url, false, false);
    assert!(!waiting.attempt_db_reconnect(&timeout, None).await);
    assert_eq!(fakes::recorded(&effects), vec![Effect::LockAcquire]);
    let effects = fakes::effects();
    let engine = FakeEngine::new(EngineScript::default(), Arc::clone(&effects));
    let failing = AuthReconnect::new(
        engine,
        settings(),
        ScriptedClock::from_seconds(&[1000.0]),
        FakeLock::new(LockMode::Error, Arc::clone(&effects)),
        ReconnectHistory::default(),
    );
    let unavailable = request("lock_error", &url, false, false);
    assert!(!failing.attempt_db_reconnect(&unavailable, None).await);
    assert_eq!(fakes::recorded(&effects), vec![Effect::LockAcquire]);
    let debug = logged(&logs, tracing::Level::DEBUG);
    assert!(
        debug
            .iter()
            .any(|fields| answers(fields, "reason", "lock_timeout"))
    );
    assert!(debug.iter().any(|fields| {
        answers(fields, "reason", "lock_error")
            && answers(fields, "error", "the reconnect lock is unavailable")
    }));
}

#[tokio::test]
async fn cancellation_acquire_race_releases() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.probes.push_back(ProbeOutcome::Value(true));
    script.probes.push_back(ProbeOutcome::Value(true));
    script.recreates.push_back(RecreateOutcome::Pending);
    script.recreates.push_back(RecreateOutcome::Value(true));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = Arc::new(AuthReconnect::new(
        engine,
        settings(),
        ScriptedClock::from_seconds(&[1000.0]),
        TokioLock::default(),
        ReconnectHistory::default(),
    ));
    let url = SecretString::from(URL);
    let first = {
        let policy = Arc::clone(&policy);
        let url = url.clone();
        tokio::spawn(async move {
            policy
                .attempt_db_reconnect(&request("cancelled", &url, true, false), None)
                .await
        })
    };
    while !fakes::recorded(&effects)
        .iter()
        .any(|effect| matches!(effect, Effect::Recreate { .. }))
    {
        tokio::task::yield_now().await;
    }
    first.abort();
    assert!(first.await.is_err());
    let mut after = request("after_cancel", &url, true, false);
    after.lock_timeout = Some(Duration::from_millis(20));
    assert!(policy.attempt_db_reconnect(&after, None).await);
}

#[tokio::test]
async fn escalation_marks_dead() {
    let logs = CapturedLogs::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.probes.push_back(ProbeOutcome::Value(true));
    script.probes.push_back(ProbeOutcome::Value(true));
    script.probes.push_back(ProbeOutcome::Value(true));
    script
        .recreates
        .push_back(RecreateOutcome::Failure("recreate failed".to_owned()));
    script
        .recreates
        .push_back(RecreateOutcome::Failure("recreate failed".to_owned()));
    script
        .recreates
        .push_back(RecreateOutcome::Failure("recreate failed".to_owned()));
    script
        .recreates
        .push_back(RecreateOutcome::Failure("recreate failed".to_owned()));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    seed_failure(&policy, &url, "seeding").await;
    assert_eq!(policy.snapshot().consecutive_failures, 1);
    seed_failure(&policy, &url, "seeding").await;
    assert_eq!(policy.snapshot().consecutive_failures, 2);
    seed_failure(&policy, &url, "seeding").await;
    assert_eq!(policy.snapshot().consecutive_failures, 3);
    let escalated = request("escalated", &url, true, false);
    assert!(!policy.attempt_db_reconnect(&escalated, None).await);
    assert!(policy.snapshot().engine_dead);
    let recorded = fakes::recorded(&effects);
    let probes = recorded
        .iter()
        .filter(|effect| **effect == Effect::Probe)
        .count();
    let recreates = recorded
        .iter()
        .filter(|effect| matches!(effect, Effect::Recreate { .. }))
        .count();
    assert_eq!(probes, 3);
    assert_eq!(recreates, 4);
    assert!(recorded.contains(&Effect::Reap));
    let warnings = logged(&logs, tracing::Level::WARN);
    assert!(warnings.iter().any(|fields| {
        answers(fields, "reason", "escalated") && answers(fields, "consecutive", "3")
    }));
}

#[tokio::test]
async fn generation_snapshotted_before_await() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.bump_on_probe = true;
    script.probes.push_back(ProbeOutcome::Value(true));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine.clone(), Arc::clone(&effects));
    let url = SecretString::from(URL);
    let snapshot = request("snapshot", &url, false, false);
    assert!(policy.attempt_db_reconnect(&snapshot, None).await);
    assert_eq!(engine.writer_observation().generation, 1);
    assert!(fakes::recorded(&effects).contains(&Effect::Recreate {
        expected_generation: Some(0),
    }));
}

#[tokio::test]
async fn missing_database_url_error() {
    let logs = CapturedLogs::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
    let effects = fakes::effects();
    let engine = FakeEngine::new(EngineScript::default(), Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let missing = ReconnectRequest {
        reason: "missing_url",
        force: false,
        force_recreate: false,
        timeout: None,
        lock_timeout: None,
        database_url: None,
    };
    assert!(!policy.attempt_db_reconnect(&missing, None).await);
    assert_eq!(policy.snapshot().consecutive_failures, 1);
    assert_eq!(
        fakes::recorded(&effects),
        vec![Effect::LockAcquire, Effect::LockRelease]
    );
    let errors = logged(&logs, tracing::Level::ERROR);
    assert!(errors.iter().any(|fields| {
        answers(fields, "reason", "missing_url") && answers(fields, "error", "DATABASE_URL not set")
    }));
}

#[tokio::test]
async fn dead_recreate_failure_keeps_marker() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.probes.push_back(ProbeOutcome::Value(true));
    script.probes.push_back(ProbeOutcome::Value(true));
    script.probes.push_back(ProbeOutcome::Value(true));
    script
        .recreates
        .push_back(RecreateOutcome::Failure("seeding".to_owned()));
    script
        .recreates
        .push_back(RecreateOutcome::Failure("seeding".to_owned()));
    script
        .recreates
        .push_back(RecreateOutcome::Failure("seeding".to_owned()));
    script
        .recreates
        .push_back(RecreateOutcome::Failure("dead".to_owned()));
    script.recreates.push_back(RecreateOutcome::Value(true));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    seed_failure(&policy, &url, "seeding").await;
    seed_failure(&policy, &url, "seeding").await;
    seed_failure(&policy, &url, "seeding").await;
    let dead = request("dead_recreate_fails", &url, true, false);
    assert!(!policy.attempt_db_reconnect(&dead, None).await);
    assert!(policy.snapshot().engine_dead);
    let recovers = request("dead_recreate_recovers", &url, true, false);
    assert!(policy.attempt_db_reconnect(&recovers, None).await);
    assert!(!policy.snapshot().engine_dead);
}

#[tokio::test]
async fn direct_writable_short_circuit() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.routing = true;
    script.probes.push_back(ProbeOutcome::Value(false));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    let writable = request("writable", &url, false, false);
    assert!(policy.attempt_db_reconnect(&writable, None).await);
    assert_eq!(
        fakes::recorded(&effects),
        vec![
            Effect::LockAcquire,
            Effect::Probe,
            Effect::MarkWriterRecovered,
            Effect::Rearm,
            Effect::LockRelease,
        ]
    );
}

#[rstest]
#[case::read_only_session(ProbeOutcome::Value(true), false)]
#[case::probe_error(ProbeOutcome::Failure("probe failed".to_owned()), true)]
#[tokio::test]
async fn readonly_or_probe_error_recreates(#[case] probe: ProbeOutcome, #[case] failed: bool) {
    let logs = CapturedLogs::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.probes.push_back(probe);
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    let recreate = request("recreate", &url, false, false);
    assert!(policy.attempt_db_reconnect(&recreate, None).await);
    assert_eq!(
        fakes::recorded(&effects),
        vec![
            Effect::LockAcquire,
            Effect::Probe,
            Effect::Cleanup,
            Effect::Recreate {
                expected_generation: Some(0),
            },
            Effect::Rearm,
            Effect::SelectOne,
            Effect::LockRelease,
        ]
    );
    let warnings = logged(&logs, tracing::Level::WARN);
    assert_eq!(
        warnings
            .iter()
            .any(|fields| answers(fields, "error", "probe failed")),
        failed
    );
}

#[tokio::test]
async fn forced_skips_probe() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script
        .probes
        .push_back(ProbeOutcome::Failure("the probe must not run".to_owned()));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    let forced = request("forced", &url, false, true);
    assert!(policy.attempt_db_reconnect(&forced, None).await);
    let recorded = fakes::recorded(&effects);
    assert!(!recorded.contains(&Effect::Probe));
    assert!(recorded.contains(&Effect::Recreate {
        expected_generation: Some(0),
    }));
}

#[tokio::test]
async fn direct_select_one_after_recreate() {
    let logs = CapturedLogs::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.probes.push_back(ProbeOutcome::Value(true));
    script
        .selects
        .push_back(StepOutcome::Failure("select one failed".to_owned()));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    let select_fails = request("select_fails", &url, false, false);
    assert!(!policy.attempt_db_reconnect(&select_fails, None).await);
    assert_eq!(policy.snapshot().consecutive_failures, 1);
    let recorded = fakes::recorded(&effects);
    let rearm = recorded.iter().position(|effect| *effect == Effect::Rearm);
    let select = recorded
        .iter()
        .position(|effect| *effect == Effect::SelectOne);
    assert!(rearm < select);
    let errors = logged(&logs, tracing::Level::ERROR);
    assert!(
        errors
            .iter()
            .any(|fields| answers(fields, "error", "select one failed"))
    );
}

#[tokio::test]
async fn forced_decline_false_resets_failures() {
    let logs = CapturedLogs::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.probes.push_back(ProbeOutcome::Value(true));
    script.probes.push_back(ProbeOutcome::Value(true));
    script.probes.push_back(ProbeOutcome::Value(true));
    script
        .recreates
        .push_back(RecreateOutcome::Failure("first".to_owned()));
    script
        .recreates
        .push_back(RecreateOutcome::Failure("second".to_owned()));
    script.recreates.push_back(RecreateOutcome::Value(false));
    script
        .recreates
        .push_back(RecreateOutcome::Failure("after".to_owned()));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    seed_failure(&policy, &url, "seeding").await;
    seed_failure(&policy, &url, "seeding").await;
    assert_eq!(policy.snapshot().consecutive_failures, 2);
    let declined = request("declined", &url, true, true);
    assert!(!policy.attempt_db_reconnect(&declined, None).await);
    assert_eq!(policy.snapshot().consecutive_failures, 0);
    let after = request("after_decline", &url, true, false);
    assert!(!policy.attempt_db_reconnect(&after, None).await);
    let errors = logged(&logs, tracing::Level::ERROR);
    assert!(
        errors
            .iter()
            .any(|fields| answers(fields, "reason", "after_decline")
                && answers(fields, "consecutive", "1"))
    );
    let warnings = logged(&logs, tracing::Level::WARN);
    assert!(
        !warnings
            .iter()
            .any(|fields| answers(fields, "reason", "escalated"))
    );
    assert!(
        warnings
            .iter()
            .any(|fields| answers(fields, "reason", "declined"))
    );
}

#[tokio::test]
async fn failures_record_original_target() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.routing = true;
    script.probes.push_back(ProbeOutcome::Value(true));
    script
        .recreates
        .push_back(RecreateOutcome::Failure("recreate failed".to_owned()));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine.clone(), Arc::clone(&effects));
    let url = SecretString::from(URL);
    let stale =
        StaleReadEngine::observe(&engine.writer_observation()).expect("the writer is named");
    let failed = request("writer_failed", &url, true, false);
    assert!(!policy.attempt_db_reconnect(&failed, Some(&stale)).await);
    assert!(policy.has_failed_repair_for(&engine.writer_handle(), 0));
    assert!(!policy.has_failed_repair_for(&engine.reader_handle(), 0));
}

#[tokio::test(start_paused = true)]
async fn overall_timeout() {
    let logs = CapturedLogs::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.probes.push_back(ProbeOutcome::Value(true));
    script.recreates.push_back(RecreateOutcome::Pending);
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    let mut attempt = request("timeout", &url, false, false);
    attempt.timeout = Some(Duration::from_millis(1));
    assert!(!policy.attempt_db_reconnect(&attempt, None).await);
    assert_eq!(policy.snapshot().consecutive_failures, 1);
    let recorded = fakes::recorded(&effects);
    assert!(recorded.contains(&Effect::Recreate {
        expected_generation: Some(0),
    }));
    assert!(!recorded.contains(&Effect::Rearm));
    let errors = logged(&logs, tracing::Level::ERROR);
    assert!(
        errors
            .iter()
            .any(|fields| { answers(fields, "error", "the database reconnect timed out") })
    );
    let attempts = fakes::recorded(&effects).len();
    let cooldown = request("cooldown", &url, false, false);
    assert!(!policy.attempt_db_reconnect(&cooldown, None).await);
    assert_eq!(fakes::recorded(&effects).len(), attempts);
}

#[tokio::test]
async fn dead_engine_reads_the_engine_state() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.engine_state = EngineState::Dead;
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    let dead = request("dead", &url, true, false);
    assert!(policy.attempt_db_reconnect(&dead, None).await);
    let recorded = fakes::recorded(&effects);
    assert!(recorded.contains(&Effect::Reap));
    assert!(!recorded.contains(&Effect::Probe));
}
