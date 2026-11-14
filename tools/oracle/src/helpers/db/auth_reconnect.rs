use betterlite_db::auth_reconnect::engine::EngineState;
use serde::{Deserialize, Serialize};

mod fakes;
mod scenario;
mod scripted;

use fakes::Effect;
use scenario::run_scenario;

pub(crate) struct Op;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(transparent)]
struct Flag(bool);

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
    dead: Flag,
    failed: Vec<FailedInput>,
    database_url: Option<String>,
    generations: Generations,
    reader_unavailable: bool,
    routing: bool,
    engine_state: EngineStateInput,
    bump_on_probe: Flag,
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
    result: Flag,
    effects: Vec<Effect>,
    failures: u32,
    dead: Flag,
    failed_writer: Flag,
    failed_reader: Flag,
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
