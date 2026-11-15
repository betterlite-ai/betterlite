#![cfg(test)]

mod fakes;
mod postgres;

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use rstest::rstest;
use secrecy::SecretString;
use tokio::sync::Notify;
use tracing_subscriber::layer::SubscriberExt as _;

use super::engine::EngineState;
use super::lock::TokioLock;
use super::policy::AuthReconnect;
use super::request::{ReconnectHistory, ReconnectRequest};
use super::settings::ReconnectSettings;
use super::stale::StaleReadEngine;

use fakes::{
    CapturedLogs, Effect, Effects, EngineScript, FakeEngine, FakeLock, LockMode, ReadRoute,
    ScriptedClock, Step,
};

const URL: &str = "postgresql://user:pw@db.example:5432/litellm";

type Policy = AuthReconnect<FakeEngine, ScriptedClock, FakeLock>;
type GatedPolicy = AuthReconnect<FakeEngine, ScriptedClock, TokioLock>;

const fn settings() -> ReconnectSettings {
    ReconnectSettings {
        cooldown: Duration::from_secs(15),
        escalation_threshold: 3,
        watchdog_timeout: Duration::from_secs(30),
        auth_timeout: Duration::from_secs(2),
        auth_lock_timeout: Duration::from_millis(100),
    }
}

fn policy_with_engine(engine: FakeEngine, effects: Effects) -> Policy {
    let lock = FakeLock::new(LockMode::Free, effects);
    policy_of(engine, lock)
}

fn policy_with_lock(mode: LockMode, effects: &Effects) -> Policy {
    let engine = FakeEngine::new(EngineScript::default(), Arc::clone(effects));
    let lock = FakeLock::new(mode, Arc::clone(effects));
    policy_of(engine, lock)
}

fn policy_of(engine: FakeEngine, lock: FakeLock) -> Policy {
    AuthReconnect::new(
        engine,
        settings(),
        ScriptedClock::from_seconds(&[1000.0]),
        lock,
        ReconnectHistory::default(),
    )
}

fn gated_policy(effects: &Effects, gate: &Arc<Notify>) -> GatedPolicy {
    let mut script = EngineScript::default();
    script.probes.push_back(Step::Done(true));
    script.probes.push_back(Step::Done(true));
    script.recreates.push_back(Step::Done(true));
    script.hold_recreate = Some(Arc::clone(gate));
    AuthReconnect::new(
        FakeEngine::new(script, Arc::clone(effects)),
        settings(),
        ScriptedClock::from_seconds(&[1000.0]),
        TokioLock::default(),
        ReconnectHistory::default(),
    )
}

fn scripted(probes: &[bool], recreates: Vec<Step<bool>>) -> EngineScript {
    let mut script = EngineScript::default();
    script.probes = probes.iter().copied().map(Step::Done).collect();
    script.recreates = VecDeque::from(recreates);
    script
}

fn escalation_script() -> EngineScript {
    scripted(
        &[true, true, true],
        vec![Step::Failure("recreate failed".to_owned()); 4],
    )
}

fn decline_script() -> EngineScript {
    scripted(
        &[true, true, true, true],
        vec![
            Step::Failure("first".to_owned()),
            Step::Failure("second".to_owned()),
            Step::Done(false),
            Step::Failure("after".to_owned()),
        ],
    )
}

async fn seed_two_failures(policy: &Policy, url: &SecretString) {
    seed_failure(policy, url, "seeding").await;
    seed_failure(policy, url, "seeding").await;
    assert_eq!(policy.snapshot().consecutive_failures, 2);
}

async fn seed_three_failures(policy: &Policy, url: &SecretString) {
    seed_failure(policy, url, "seeding").await;
    assert_eq!(policy.snapshot().consecutive_failures, 1);
    seed_failure(policy, url, "seeding").await;
    assert_eq!(policy.snapshot().consecutive_failures, 2);
    seed_failure(policy, url, "seeding").await;
    assert_eq!(policy.snapshot().consecutive_failures, 3);
}

async fn seed_failure(policy: &Policy, url: &SecretString, reason: &str) {
    let seeding = request(reason, url).forced();
    assert!(!policy.attempt_db_reconnect(&seeding, None).await);
}

async fn wait_for_recreate(effects: &Effects) {
    while !fakes::recorded(effects)
        .iter()
        .any(|effect| matches!(effect, Effect::Recreate { .. }))
    {
        tokio::task::yield_now().await;
    }
}

const fn request<'request>(
    reason: &'request str,
    url: &'request SecretString,
) -> ReconnectRequest<'request> {
    ReconnectRequest {
        reason,
        force: false,
        force_recreate: false,
        timeout: None,
        lock_timeout: None,
        database_url: Some(url),
    }
}

impl ReconnectRequest<'_> {
    #[must_use]
    const fn forced(mut self) -> Self {
        self.force = true;
        self
    }

    #[must_use]
    const fn recreating(mut self) -> Self {
        self.force_recreate = true;
        self
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
    script.read_route = ReadRoute::Reader;
    script.probes.push_back(Step::Done(false));
    script.probes.push_back(Step::Done(false));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine.clone(), Arc::clone(&effects));
    let url = SecretString::from(URL);
    let live = StaleReadEngine::observe(&engine.reader_observation()).expect("the reader is named");
    let first = request("first", &url);
    assert!(policy.attempt_db_reconnect(&first, Some(&live)).await);
    // the reader is still live, so its own repair waives the cooldown it just stamped
    let waived = request("live", &url);
    assert!(policy.attempt_db_reconnect(&waived, Some(&live)).await);
    // the writer carries the same generation number, so only the handle says it was replaced
    let switched =
        StaleReadEngine::observe(&engine.writer_observation()).expect("the writer is named");
    let attempts = fakes::recorded(&effects).len();
    let gated = request("switched", &url);
    assert!(!policy.attempt_db_reconnect(&gated, Some(&switched)).await);
    assert_eq!(fakes::recorded(&effects).len(), attempts);
}

#[tokio::test]
async fn stale_live_waives_once() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.read_route = ReadRoute::Reader;
    script.probes.push_back(Step::Done(false));
    script.probes.push_back(Step::Done(true));
    script
        .recreates
        .push_back(Step::Failure("recreate failed".to_owned()));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine.clone(), Arc::clone(&effects));
    let url = SecretString::from(URL);
    let stale =
        StaleReadEngine::observe(&engine.reader_observation()).expect("the reader is named");
    let first = request("first_attempt", &url);
    assert!(policy.attempt_db_reconnect(&first, Some(&stale)).await);
    // the live engine has no failed repair yet, so the cooldown does not gate its own repair
    let waived = request("repair_fails", &url);
    assert!(!policy.attempt_db_reconnect(&waived, Some(&stale)).await);
    assert!(policy.has_failed_repair_for(&engine.reader_handle(), 0));
    // the failed repair of this engine now makes the same live observation respect the cooldown
    let attempts = fakes::recorded(&effects).len();
    let recorded = request("recorded_engine", &url);
    assert!(!policy.attempt_db_reconnect(&recorded, Some(&stale)).await);
    assert_eq!(fakes::recorded(&effects).len(), attempts);
    let forced = request("forced", &url).forced();
    assert!(policy.attempt_db_reconnect(&forced, Some(&stale)).await);
}

#[tokio::test]
async fn failed_repair_cooldown() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.probes.push_back(Step::Done(true));
    script
        .recreates
        .push_back(Step::Failure("recreate failed".to_owned()));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine.clone(), Arc::clone(&effects));
    let url = SecretString::from(URL);
    let stale =
        StaleReadEngine::observe(&engine.writer_observation()).expect("the writer is named");
    let repair = request("forced_repair", &url).forced();
    assert!(!policy.attempt_db_reconnect(&repair, Some(&stale)).await);
    let attempts = fakes::recorded(&effects).len();
    let cooldown = request("cooldown", &url);
    assert!(!policy.attempt_db_reconnect(&cooldown, Some(&stale)).await);
    assert_eq!(fakes::recorded(&effects).len(), attempts);
}

#[tokio::test]
async fn force_bypasses_both_checks() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.probes.push_back(Step::Done(true));
    script
        .recreates
        .push_back(Step::Failure("recreate failed".to_owned()));
    script.probes.push_back(Step::Done(false));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine.clone(), Arc::clone(&effects));
    let url = SecretString::from(URL);
    let stale =
        StaleReadEngine::observe(&engine.writer_observation()).expect("the writer is named");
    let first = request("first", &url);
    assert!(!policy.attempt_db_reconnect(&first, Some(&stale)).await);
    let attempts = fakes::recorded(&effects).len();
    let cooldown = request("cooldown", &url);
    assert!(!policy.attempt_db_reconnect(&cooldown, Some(&stale)).await);
    assert_eq!(fakes::recorded(&effects).len(), attempts);
    let forced = request("forced", &url).forced();
    assert!(policy.attempt_db_reconnect(&forced, Some(&stale)).await);
    assert!(fakes::recorded(&effects).len() > attempts);
}

#[tokio::test]
async fn queued_caller_rechecks() {
    let effects = fakes::effects();
    let gate = Arc::new(Notify::new());
    let policy = gated_policy(&effects, &gate);
    let url = SecretString::from(URL);
    let first_request = request("first", &url);
    let first = policy.attempt_db_reconnect(&first_request, None);
    tokio::pin!(first);
    tokio::select! {
        biased;
        result = first.as_mut() => {
            panic!("the first caller must wait on the recreate gate, got {result}");
        }
        () = wait_for_recreate(&effects) => {}
    }
    let second_request = request("queued", &url);
    let second = policy.attempt_db_reconnect(&second_request, None);
    tokio::pin!(second);
    tokio::select! {
        biased;
        result = second.as_mut() => {
            panic!("the queued caller must wait for the lock, got {result}");
        }
        () = tokio::task::yield_now() => {}
    }
    gate.notify_waiters();
    assert!(first.await);
    assert!(!second.await);
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
    script.probes.push_back(Step::Done(false));
    script.probes.push_back(Step::Done(false));
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
    let first = request("first", &url);
    assert!(policy.attempt_db_reconnect(&first, None).await);
    // a stamp at entry (0s) would let this one through; the completion stamp (10s) does not
    let within = request("within_window", &url);
    assert!(!policy.attempt_db_reconnect(&within, None).await);
    let elapsed = request("window_elapsed", &url);
    assert!(policy.attempt_db_reconnect(&elapsed, None).await);
}

#[tokio::test]
async fn lock_timeout_and_acquire_error() {
    let logs = CapturedLogs::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
    let url = SecretString::from(URL);
    let timeout_effects = fakes::effects();
    let waiting = policy_with_lock(LockMode::Timeout, &timeout_effects);
    let timeout = request("lock_timeout", &url);
    assert!(!waiting.attempt_db_reconnect(&timeout, None).await);
    assert_eq!(fakes::recorded(&timeout_effects), vec![Effect::LockAcquire]);
    let error_effects = fakes::effects();
    let failing = policy_with_lock(LockMode::Error, &error_effects);
    let unavailable = request("lock_error", &url);
    assert!(!failing.attempt_db_reconnect(&unavailable, None).await);
    assert_eq!(fakes::recorded(&error_effects), vec![Effect::LockAcquire]);
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
    script.probes.push_back(Step::Done(true));
    script.probes.push_back(Step::Done(true));
    script.recreates.push_back(Step::Pending);
    script.recreates.push_back(Step::Done(true));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = AuthReconnect::new(
        engine,
        settings(),
        ScriptedClock::from_seconds(&[1000.0]),
        TokioLock::default(),
        ReconnectHistory::default(),
    );
    let url = SecretString::from(URL);
    let cancelled = request("cancelled", &url).forced();
    tokio::select! {
        biased;
        result = policy.attempt_db_reconnect(&cancelled, None) => {
            panic!("the cancelled caller must wait on the recreate gate, got {result}");
        }
        () = wait_for_recreate(&effects) => {}
    }
    let mut after = request("after_cancel", &url).forced();
    after.lock_timeout = Some(Duration::from_millis(20));
    assert!(policy.attempt_db_reconnect(&after, None).await);
}

#[tokio::test]
async fn escalation_marks_dead() {
    let logs = CapturedLogs::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
    let effects = fakes::effects();
    let engine = FakeEngine::new(escalation_script(), Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    seed_three_failures(&policy, &url).await;
    let escalated = request("escalated", &url).forced();
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
    script.probes.push_back(Step::Done(true));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine.clone(), Arc::clone(&effects));
    let url = SecretString::from(URL);
    let snapshot = request("snapshot", &url);
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
async fn empty_database_url_error() {
    let effects = fakes::effects();
    let engine = FakeEngine::new(EngineScript::default(), Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let empty = SecretString::from("");
    let attempt = request("empty_url", &empty);
    assert!(!policy.attempt_db_reconnect(&attempt, None).await);
    assert_eq!(policy.snapshot().consecutive_failures, 1);
    assert_eq!(
        fakes::recorded(&effects),
        vec![Effect::LockAcquire, Effect::LockRelease]
    );
}

#[tokio::test]
async fn dead_recreate_failure_keeps_marker() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.probes.push_back(Step::Done(true));
    script.probes.push_back(Step::Done(true));
    script.probes.push_back(Step::Done(true));
    script
        .recreates
        .push_back(Step::Failure("seeding".to_owned()));
    script
        .recreates
        .push_back(Step::Failure("seeding".to_owned()));
    script
        .recreates
        .push_back(Step::Failure("seeding".to_owned()));
    script.recreates.push_back(Step::Failure("dead".to_owned()));
    script.recreates.push_back(Step::Done(true));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    seed_failure(&policy, &url, "seeding").await;
    seed_failure(&policy, &url, "seeding").await;
    seed_failure(&policy, &url, "seeding").await;
    let dead = request("dead_recreate_fails", &url).forced();
    assert!(!policy.attempt_db_reconnect(&dead, None).await);
    assert!(policy.snapshot().engine_dead);
    let recovers = request("dead_recreate_recovers", &url).forced();
    assert!(policy.attempt_db_reconnect(&recovers, None).await);
    assert!(!policy.snapshot().engine_dead);
}

#[tokio::test]
async fn direct_writable_short_circuit() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.read_route = ReadRoute::Reader;
    script.probes.push_back(Step::Done(false));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    let writable = request("writable", &url);
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
#[case::read_only_session(Step::Done(true), false)]
#[case::probe_error(Step::Failure("probe failed".to_owned()), true)]
#[tokio::test]
async fn readonly_or_probe_error_recreates(#[case] probe: Step<bool>, #[case] failed: bool) {
    let logs = CapturedLogs::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.probes.push_back(probe);
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    let recreate = request("recreate", &url);
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
        .push_back(Step::Failure("the probe must not run".to_owned()));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    let forced = request("forced", &url).recreating();
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
    script.probes.push_back(Step::Done(true));
    script
        .selects
        .push_back(Step::Failure("select one failed".to_owned()));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    let select_fails = request("select_fails", &url);
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
    let engine = FakeEngine::new(decline_script(), Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    seed_two_failures(&policy, &url).await;
    let declined = request("declined", &url).forced().recreating();
    assert!(!policy.attempt_db_reconnect(&declined, None).await);
    assert_eq!(policy.snapshot().consecutive_failures, 0);
    let after = request("after_decline", &url).forced();
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
    script.read_route = ReadRoute::Reader;
    script.probes.push_back(Step::Done(true));
    script
        .recreates
        .push_back(Step::Failure("recreate failed".to_owned()));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine.clone(), Arc::clone(&effects));
    let url = SecretString::from(URL);
    let stale =
        StaleReadEngine::observe(&engine.writer_observation()).expect("the writer is named");
    let failed = request("writer_failed", &url).forced();
    assert!(!policy.attempt_db_reconnect(&failed, Some(&stale)).await);
    assert!(policy.has_failed_repair_for(&engine.writer_handle(), 0));
    assert!(!policy.has_failed_repair_for(&engine.reader_handle(), 0));
}

#[tokio::test]
async fn failed_repairs_keep_two() {
    let effects = fakes::effects();
    let engine = FakeEngine::new(EngineScript::default(), Arc::clone(&effects));
    let policy = policy_with_engine(engine.clone(), Arc::clone(&effects));
    let other = FakeEngine::new(EngineScript::default(), fakes::effects());
    let first =
        StaleReadEngine::observe(&engine.writer_observation()).expect("the writer is named");
    let second =
        StaleReadEngine::observe(&engine.reader_observation()).expect("the reader is named");
    let third = StaleReadEngine::observe(&other.writer_observation()).expect("the writer is named");
    let missing = ReconnectRequest {
        reason: "missing_url",
        force: true,
        force_recreate: false,
        timeout: None,
        lock_timeout: None,
        database_url: None,
    };
    assert!(!policy.attempt_db_reconnect(&missing, Some(&first)).await);
    assert!(!policy.attempt_db_reconnect(&missing, Some(&second)).await);
    assert!(!policy.attempt_db_reconnect(&missing, Some(&third)).await);
    assert!(!policy.has_failed_repair_for(&engine.writer_handle(), 0));
    assert!(policy.has_failed_repair_for(&engine.reader_handle(), 0));
    assert!(policy.has_failed_repair_for(&other.writer_handle(), 0));
}

#[tokio::test(start_paused = true)]
async fn overall_timeout() {
    let logs = CapturedLogs::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(logs.clone()));
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.probes.push_back(Step::Done(true));
    script.recreates.push_back(Step::Pending);
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = policy_with_engine(engine, Arc::clone(&effects));
    let url = SecretString::from(URL);
    let mut attempt = request("timeout", &url);
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
    let cooldown = request("cooldown", &url);
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
    let dead = request("dead", &url).forced();
    assert!(policy.attempt_db_reconnect(&dead, None).await);
    let recorded = fakes::recorded(&effects);
    assert!(recorded.contains(&Effect::Reap));
    assert!(!recorded.contains(&Effect::Probe));
}

#[tokio::test]
async fn dead_forced_decline_clears_marker() {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.recreates.push_back(Step::Done(false));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = AuthReconnect::new(
        engine,
        settings(),
        ScriptedClock::from_seconds(&[1000.0]),
        FakeLock::new(LockMode::Free, Arc::clone(&effects)),
        ReconnectHistory {
            last_attempt: None,
            consecutive_failures: 0,
            engine_dead: true,
            failed_repairs: Vec::new(),
        },
    );
    let url = SecretString::from(URL);
    let declined = request("dead_declined", &url).forced().recreating();
    assert!(!policy.attempt_db_reconnect(&declined, None).await);
    assert!(!policy.snapshot().engine_dead);
    assert_eq!(policy.snapshot().consecutive_failures, 0);
}

#[rstest]
#[case::dead_engine(true)]
#[case::live_engine(false)]
#[tokio::test]
async fn unforced_false_replacement_succeeds(#[case] dead: bool) {
    let effects = fakes::effects();
    let mut script = EngineScript::default();
    script.probes.push_back(Step::Done(true));
    script.recreates.push_back(Step::Done(false));
    let engine = FakeEngine::new(script, Arc::clone(&effects));
    let policy = AuthReconnect::new(
        engine,
        settings(),
        ScriptedClock::from_seconds(&[1000.0]),
        FakeLock::new(LockMode::Free, Arc::clone(&effects)),
        ReconnectHistory {
            last_attempt: None,
            consecutive_failures: 0,
            engine_dead: dead,
            failed_repairs: Vec::new(),
        },
    );
    let url = SecretString::from(URL);
    let attempt = request("unforced", &url).forced();
    assert!(policy.attempt_db_reconnect(&attempt, None).await);
    assert_eq!(policy.snapshot().consecutive_failures, 0);
}
