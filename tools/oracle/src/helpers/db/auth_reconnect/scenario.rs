use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_db::auth_reconnect::policy::AuthReconnect;
use betterlite_db::auth_reconnect::request::{ReconnectHistory, ReconnectRequest};
use betterlite_db::auth_reconnect::settings::ReconnectSettings;
use betterlite_db::auth_reconnect::stale::StaleReadEngine;
use secrecy::SecretString;
use tracing_subscriber::layer::SubscriberExt as _;

use super::fakes::{self, Database, FakeEngine};
use super::scripted::{Capture, ScriptedClock, ScriptedLock};
use super::{Flag, Report, Scenario, TargetName, failure};

type ScenarioPolicy = AuthReconnect<FakeEngine, ScriptedClock, ScriptedLock>;

fn scenario_engine(scenario: &Scenario, effects: &fakes::Effects) -> FakeEngine {
    let database = match (scenario.routing, scenario.reader_unavailable) {
        (false, _) => Database::WriterOnly,
        (true, false) => Database::Reader,
        (true, true) => Database::UnavailableReader,
    };
    FakeEngine::new(
        fakes::EngineScript {
            writer_generation: scenario.generations.writer,
            reader_generation: scenario.generations.reader,
            database,
            engine_state: scenario.engine_state.state(),
            probe: scenario.writer_read_only.clone(),
            recreate: scenario.recreate.clone(),
            select: scenario.select_one.clone(),
            bump_on_probe: scenario.bump_writer_generation_on_probe.0,
        },
        Arc::clone(effects),
    )
}

const fn scenario_settings(scenario: &Scenario) -> ReconnectSettings {
    ReconnectSettings {
        cooldown: Duration::from_secs(scenario.settings.cooldown_seconds),
        escalation_threshold: scenario.settings.escalation_threshold,
        watchdog_timeout: Duration::from_millis(scenario.settings.watchdog_timeout_ms),
        auth_timeout: Duration::from_millis(scenario.settings.auth_timeout_ms),
        auth_lock_timeout: Duration::from_millis(scenario.settings.auth_lock_timeout_ms),
    }
}

fn scenario_history(
    scenario: &Scenario,
    last_attempt: Duration,
    clock: &ScriptedClock,
    engine: &FakeEngine,
) -> ReconnectHistory {
    let failed_repairs = scenario
        .failed
        .iter()
        .map(|repair| (engine.report_handle(repair.target), repair.generation))
        .collect();
    ReconnectHistory {
        last_attempt: Some(clock.at(last_attempt)),
        consecutive_failures: scenario.failures,
        engine_dead: scenario.dead.0,
        failed_repairs,
    }
}

fn scenario_report(
    scenario: &Scenario,
    policy: &ScenarioPolicy,
    engine: &FakeEngine,
    effects: &fakes::Effects,
    result: bool,
) -> Report {
    let reader_target = if scenario.routing {
        TargetName::Reader
    } else {
        TargetName::Writer
    };
    let snapshot = policy.snapshot();
    let recorded = effects
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Report {
        name: scenario.name.clone(),
        result: Flag(result),
        effects: recorded,
        failures: snapshot.consecutive_failures,
        dead: Flag(snapshot.engine_dead),
        failed_writer: Flag(policy.has_failed_repair_for(
            &engine.report_handle(TargetName::Writer),
            engine.current_generation(TargetName::Writer),
        )),
        failed_reader: Flag(policy.has_failed_repair_for(
            &engine.report_handle(reader_target),
            engine.current_generation(reader_target),
        )),
    }
}

pub(super) fn run_scenario(
    scenario: &Scenario,
    runtime: &tokio::runtime::Runtime,
) -> Result<Report, Box<crate::helpers::HelperError>> {
    let effects = fakes::effects();
    let engine = scenario_engine(scenario, &effects);
    let clock = ScriptedClock::from_values(&scenario.clock)?;
    let lock = ScriptedLock::new(&scenario.lock, Arc::clone(&effects));
    let last_attempt = Duration::try_from_secs_f64(scenario.last_attempt)
        .map_err(|_error| failure("last_attempt is nonnegative seconds"))?;
    let history = scenario_history(scenario, last_attempt, &clock, &engine);
    let policy = AuthReconnect::new(
        engine.clone(),
        scenario_settings(scenario),
        clock,
        lock,
        history,
    );
    let database_url = scenario.database_url.clone().map(SecretString::from);
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
    Ok(scenario_report(
        scenario, &policy, &engine, &effects, result,
    ))
}
