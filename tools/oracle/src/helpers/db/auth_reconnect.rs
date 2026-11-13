use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_db::auth_reconnect::engine::EngineState;
use betterlite_db::auth_reconnect::policy::AuthReconnect;
use betterlite_db::auth_reconnect::request::{ReconnectHistory, ReconnectRequest};
use betterlite_db::auth_reconnect::settings::ReconnectSettings;
use betterlite_db::auth_reconnect::stale::StaleReadEngine;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tracing_subscriber::layer::SubscriberExt as _;

mod fakes;
mod scripted;

use fakes::{Effect, FakeEngine};
use scripted::{Capture, ScriptedClock, ScriptedLock};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    name: String,
    settings: SettingsInput,
    clock: Vec<f64>,
    last_attempt: f64,
    failures: u32,
    dead: bool,
    failed: Vec<FailedInput>,
    database_url: Option<String>,
    generations: Generations,
    reader_unavailable: bool,
    routing: bool,
    engine_state: EngineStateInput,
    bump_writer_generation_on_probe: bool,
    request: RequestInput,
    stale: Option<StaleInput>,
    lock: LockScript,
    recreate: Outcome,
    writer_read_only: Outcome,
    select_one: Outcome,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsInput {
    cooldown_seconds: u64,
    escalation_threshold: u64,
    watchdog_timeout_ms: u64,
    auth_timeout_ms: u64,
    auth_lock_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Outcome {
    #[serde(default)]
    pub(super) value: Option<bool>,
    #[serde(default)]
    pub(super) error: Option<String>,
    #[serde(default)]
    pub(super) pending: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestInput {
    reason: String,
    force: bool,
    force_recreate: bool,
    timeout_ms: Option<u64>,
    lock_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum TargetName {
    Writer,
    Reader,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FailedInput {
    target: TargetName,
    generation: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StaleInput {
    target: TargetName,
    generation: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Generations {
    writer: u64,
    reader: u64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EngineStateInput {
    Alive,
    Dead,
    Unknown,
}

impl EngineStateInput {
    const fn state(self) -> EngineState {
        match self {
            Self::Alive => EngineState::Alive,
            Self::Dead => EngineState::Dead,
            Self::Unknown => EngineState::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum LockMode {
    Free,
    Busy,
    Error,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LockScript {
    pub(super) mode: LockMode,
    #[serde(default)]
    pub(super) error: Option<String>,
}

#[derive(Debug, Serialize)]
struct Report {
    name: String,
    result: bool,
    effects: Vec<Effect>,
    failures: u32,
    dead: bool,
    failed_writer: bool,
    failed_reader: bool,
}

fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(failure)?;
        let reports = input
            .scenarios
            .iter()
            .map(|scenario| run_scenario(scenario, &runtime))
            .collect::<Result<Vec<_>, _>>()?;
        serde_json::to_value(reports).map_err(failure)
    }
}

fn run_scenario(
    scenario: &Scenario,
    runtime: &tokio::runtime::Runtime,
) -> Result<Report, Box<crate::helpers::HelperError>> {
    let effects = fakes::effects();
    let engine = FakeEngine::new(
        fakes::EngineScript {
            writer_generation: scenario.generations.writer,
            reader_generation: scenario.generations.reader,
            routing: scenario.routing,
            reader_unavailable: scenario.reader_unavailable,
            engine_state: scenario.engine_state.state(),
            probe: scenario.writer_read_only.clone(),
            recreate: scenario.recreate.clone(),
            select: scenario.select_one.clone(),
            bump_on_probe: scenario.bump_writer_generation_on_probe,
        },
        Arc::clone(&effects),
    );
    let clock = ScriptedClock::from_values(&scenario.clock)?;
    let lock = ScriptedLock::new(&scenario.lock, Arc::clone(&effects));
    let last_attempt = Duration::try_from_secs_f64(scenario.last_attempt)
        .map_err(|_error| failure("last_attempt is nonnegative seconds"))?;
    let failed_repairs = scenario
        .failed
        .iter()
        .map(|repair| (engine.report_handle(repair.target), repair.generation))
        .collect();
    let history = ReconnectHistory {
        last_attempt: Some(clock.at(last_attempt)),
        consecutive_failures: scenario.failures,
        engine_dead: scenario.dead,
        failed_repairs,
    };
    let settings = ReconnectSettings {
        cooldown: Duration::from_secs(scenario.settings.cooldown_seconds),
        escalation_threshold: scenario.settings.escalation_threshold,
        watchdog_timeout: Duration::from_millis(scenario.settings.watchdog_timeout_ms),
        auth_timeout: Duration::from_millis(scenario.settings.auth_timeout_ms),
        auth_lock_timeout: Duration::from_millis(scenario.settings.auth_lock_timeout_ms),
    };
    let database_url = scenario.database_url.clone().map(SecretString::from);
    let policy = AuthReconnect::new(engine.clone(), settings, clock, lock, history);
    let stale = scenario.stale.as_ref().and_then(|stale| {
        StaleReadEngine::observe(&engine.observation(stale.target, stale.generation))
    });
    let request = ReconnectRequest {
        reason: &scenario.request.reason,
        force: scenario.request.force,
        force_recreate: scenario.request.force_recreate,
        timeout: scenario.request.timeout_ms.map(Duration::from_millis),
        lock_timeout: scenario.request.lock_timeout_ms.map(Duration::from_millis),
        database_url: database_url.as_ref(),
    };
    let subscriber = tracing_subscriber::registry().with(Capture::new(Arc::clone(&effects)));
    let result = tracing::subscriber::with_default(subscriber, || {
        runtime.block_on(policy.attempt_db_reconnect(&request, stale.as_ref()))
    });
    let snapshot = policy.snapshot();
    let reader_target = if scenario.routing {
        TargetName::Reader
    } else {
        TargetName::Writer
    };
    let effects = effects
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(Report {
        name: scenario.name.clone(),
        result,
        effects,
        failures: snapshot.consecutive_failures,
        dead: snapshot.engine_dead,
        failed_writer: policy.has_failed_repair_for(
            &engine.report_handle(TargetName::Writer),
            engine.current_generation(TargetName::Writer),
        ),
        failed_reader: policy.has_failed_repair_for(
            &engine.report_handle(reader_target),
            engine.current_generation(reader_target),
        ),
    })
}
