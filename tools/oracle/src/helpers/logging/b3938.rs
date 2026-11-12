#[path = "b3938/datadog_llm_observability/tests.rs"]
mod datadog_llm_observability;

#[path = "b3938/datadog_llm_observability/support.rs"]
mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger as _;
use betterlite_logging::datadog_llm_observability::error::DatadogLlmObsError;
use betterlite_logging::datadog_llm_observability::flush::queue::flush_queue;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperError;
use crate::helpers::OrHelperError as _;
use support::{Logger, ScriptedTransport, event, logger_for};

pub(crate) struct Op;

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Scenario {
    name: String,
    environment: BTreeMap<String, String>,
    init: Init,
    params: Option<Params>,
    apm_trace_id: Option<u128>,
    replies: Vec<Reply>,
    actions: Vec<Action>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Init {
    dd_api_key: Option<String>,
    dd_site: Option<String>,
    dd_agent_host: Option<String>,
    dd_agent_port: Option<String>,
    turn_off_message_logging: Option<bool>,
    message_logging: Option<bool>,
    batch_size: Option<usize>,
    flush_interval: Option<f64>,
    global_turn_off: Option<bool>,
}

#[derive(Clone, Copy, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Params {
    turn_off_message_logging: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    #[serde(default = "default_status")]
    status: u16,
    #[serde(default)]
    body: String,
}

const fn default_status() -> u16 {
    202
}

#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
enum Action {
    Success(ActionCall),
    Failure(ActionCall),
    Flush,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionCall {
    payload: Box<StandardLoggingPayload>,
    #[serde(default)]
    metadata: Option<Json>,
    start_time: f64,
    end_time: f64,
}

async fn perform(logger: &Arc<Logger>, action: &Action) -> Result<Value, Box<HelperError>> {
    match action {
        Action::Success(call) => logger.async_log_success_event(event(call)?).await,
        Action::Failure(call) => logger.async_log_failure_event(event(call)?).await,
        Action::Flush => {
            flush_queue(logger).await;
            Ok(())
        }
    }
    .or_value_error()?;
    Ok(json!({"queue_length": logger.queue.pending().len()}))
}

fn queued(logger: &Arc<Logger>) -> Result<Vec<Value>, Box<HelperError>> {
    let mut encoded = Vec::new();
    for span in logger.queue.pending() {
        encoded.push(serde_json::to_value(span).or_value_error()?);
    }
    Ok(encoded)
}

async fn execute(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let transport = ScriptedTransport::new(scenario.replies.clone());
    let logger = match logger_for(scenario, &transport) {
        Ok(found) => found,
        Err(error) => return Ok(init_report(scenario, &error)),
    };
    let mut results = Vec::new();
    for action in &scenario.actions {
        results.push(perform(&logger, action).await?);
    }
    let pending = queued(&logger)?;
    let length = pending.len();
    let head: Vec<Value> = pending.iter().take(3).cloned().collect();
    let tail: Vec<Value> = pending.iter().rev().take(3).rev().cloned().collect();
    Ok(json!({
        "name": scenario.name,
        "init": {
            "batch_size": logger.settings.batch.batch_size,
            "intake_url": logger.settings.intake_url,
            "authenticated": logger.settings.api_key.is_some(),
            "error": Value::Null,
        },
        "results": results,
        "calls": transport.calls(),
        "queue": {"length": length, "head": head, "tail": tail},
    }))
}

fn init_report(scenario: &Scenario, error: &DatadogLlmObsError) -> Value {
    let reason = match error {
        DatadogLlmObsError::MissingApiKey => "missing_api_key",
        DatadogLlmObsError::Environment(_)
        | DatadogLlmObsError::MissingSite
        | DatadogLlmObsError::FlushInterval { .. }
        | DatadogLlmObsError::ApiKeyHeader(_)
        | DatadogLlmObsError::Encode(_)
        | DatadogLlmObsError::Transport(_)
        | DatadogLlmObsError::UnexpectedStatus { .. } => "missing_site",
    };
    json!({"name": scenario.name, "init": {"error": reason}})
}

fn run_scenarios(input: &Input) -> Result<Value, Box<HelperError>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    runtime.block_on(async {
        let mut scenarios = Vec::new();
        for scenario in &input.scenarios {
            scenarios.push(execute(scenario).await?);
        }
        Ok::<Value, Box<HelperError>>(json!({"scenarios": scenarios}))
    })
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        run_scenarios(input)
    }
}
