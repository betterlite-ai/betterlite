use std::time::Duration;

use betterlite_logging::worker::WorkerSettings;
use serde::Deserialize;
use tracing_subscriber::registry;

use crate::helpers::{Helper, HelperError, HelperResult};

use scenario::scenario;

mod scenario;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    settings: SettingsInput,
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct SettingsInput {
    #[serde(default)]
    max_queue_size: Option<usize>,
    #[serde(default)]
    concurrency: Option<usize>,
    #[serde(default)]
    timeout_milliseconds: Option<u64>,
    #[serde(default)]
    window_milliseconds: Option<u64>,
}

impl SettingsInput {
    fn resolve(&self) -> WorkerSettings {
        let defaults = WorkerSettings::default();
        WorkerSettings {
            max_queue_size: self.max_queue_size.unwrap_or(defaults.max_queue_size),
            concurrency: self.concurrency.unwrap_or(defaults.concurrency),
            event_timeout: self
                .timeout_milliseconds
                .map_or(defaults.event_timeout, Duration::from_millis),
            shutdown_drain: defaults.shutdown_drain,
            timeout_summary_window: self
                .window_milliseconds
                .map_or(defaults.timeout_summary_window, Duration::from_millis),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Step {
    Enqueue { events: Vec<EventInput> },
    Flush,
    Stop,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventInput {
    name: String,
    outcome: Outcome,
    context: String,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Success,
    Failure,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(failure)?;
        tracing::subscriber::with_default(registry(), || runtime.block_on(scenario(input)))
            .map_err(failure)
    }
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(HelperError {
        class: "WorkerTraceError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
