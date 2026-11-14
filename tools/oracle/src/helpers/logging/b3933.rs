mod normalize;
mod tests;
mod transport;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::callback_logger::CallbackLogger as _;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::datadog_logging::error::DatadogError;
use betterlite_logging::datadog_logging::logger::{DatadogDeps, DatadogLogger, uuid_id};
use betterlite_logging::datadog_logging::payloads::{DatadogLoggingKwargs, SyncCall};
use betterlite_logging::datadog_logging::settings::{DatadogInit, DatadogInitParams};
use betterlite_logging::datadog_logging::transport::Transport;
use betterlite_logging::datadog_logging::{batch, health, sync};
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use self::transport::{FixedClock, Reply, ScriptedTransport};

type HelperFailure = Box<crate::helpers::HelperError>;

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
    allow_env_credentials: Option<bool>,
}

#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
enum Action {
    Success { payload: Value },
    Failure { payload: Value },
    Flush,
    HealthCheck,
    SyncLog { payload: Value },
}

fn value_error<E>(error: E) -> HelperFailure
where
    E: std::fmt::Display,
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

async fn sync_log(logger: &Arc<DatadogLogger>, payload: &Value) -> Result<(), HelperFailure> {
    let kwargs = DatadogLoggingKwargs {
        standard_logging_object: Some(
            serde_json::from_value(payload.clone()).map_err(value_error)?,
        ),
        legacy: None,
    };
    let moment = DateTime::<FixedOffset>::from(DateTime::UNIX_EPOCH);
    let call = SyncCall {
        kwargs: &kwargs,
        response_obj: &Json::Null,
        start_time: moment,
        end_time: moment,
    };
    sync::log_success_event(logger, &call).await;
    Ok(())
}

async fn perform(logger: &Arc<DatadogLogger>, action: &Action) -> Result<Value, HelperFailure> {
    match action {
        Action::Success { payload } | Action::Failure { payload } => {
            let event = Arc::new(LoggerEvent {
                payload: Arc::new(
                    serde_json::from_value::<StandardLoggingPayload>(payload.clone())
                        .map_err(value_error)?,
                ),
                dynamic_params: Arc::new(Json::Object(IndexMap::new())),
                litellm_metadata: Arc::new(IndexMap::new()),
                stream: false,
                guardrail_snapshot: None,
            });
            if matches!(action, Action::Failure { .. }) {
                logger.async_log_failure_event(event).await
            } else {
                logger.async_log_success_event(event).await
            }
            .map_err(value_error)?;
        }
        Action::Flush => batch::flush_queue(logger).await,
        Action::HealthCheck => {
            let status = health::async_health_check(logger).await;
            return Ok(json!({"health": serde_json::to_value(status).map_err(value_error)?}));
        }
        Action::SyncLog { payload } => sync_log(logger, payload).await?,
    }
    Ok(json!({"queue_length": logger.queue.pending().len()}))
}

fn run_scenario(scenario: &Scenario) -> Result<Value, HelperFailure> {
    let init = DatadogInit {
        dd_api_key: scenario.init.dd_api_key.clone().map(SecretString::from),
        dd_site: scenario.init.dd_site.clone(),
        dd_agent_host: scenario.init.dd_agent_host.clone(),
        dd_agent_port: scenario.init.dd_agent_port.clone(),
        allow_env_credentials: scenario.init.allow_env_credentials.unwrap_or(true),
        params: DatadogInitParams {
            turn_off_message_logging: false,
        },
        use_v1: false,
    };
    let transport = Arc::new(ScriptedTransport::new(&scenario.replies));
    let shared = Arc::clone(&transport);
    let concrete: Arc<dyn Transport> = shared;
    let deps = DatadogDeps {
        transport: concrete,
        clock: Arc::new(FixedClock(Duration::from_secs(1))),
        ids: uuid_id,
        tasks: None,
    };
    let logger = match DatadogLogger::new(&init, &scenario.environment, deps) {
        Ok(logger) => logger,
        Err(error) => {
            let reason = if matches!(error, DatadogError::MissingApiKey) {
                "missing_api_key"
            } else {
                "missing_site"
            };
            return Ok(json!({"name": scenario.name, "init": {"error": reason}}));
        }
    };
    play(scenario, &logger, &transport)
}

fn play(
    scenario: &Scenario,
    logger: &Arc<DatadogLogger>,
    transport: &ScriptedTransport,
) -> Result<Value, HelperFailure> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(value_error)?;
    let results = runtime.block_on(async {
        let mut results = Vec::new();
        for action in &scenario.actions {
            results.push(perform(logger, action).await?);
        }
        Ok::<Vec<Value>, HelperFailure>(results)
    })?;
    let queued = logger.queue.pending();
    let head: Vec<Value> = queued.iter().take(3).map(normalize::payload).collect();
    let tail: Vec<Value> = queued
        .iter()
        .rev()
        .take(3)
        .rev()
        .map(normalize::payload)
        .collect();
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
        "queue": {"length": queued.len(), "head": head, "tail": tail},
    }))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut scenarios = Vec::new();
        for scenario in &input.scenarios {
            scenarios.push(run_scenario(scenario)?);
        }
        Ok(json!({"scenarios": scenarios}))
    }
}
