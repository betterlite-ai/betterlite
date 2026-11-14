mod transport;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::callback_logger::CallbackLogger as _;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::datadog::cost_management::datadog_cost_management::logger::{
    CostManagementDeps, DatadogCostManagementLogger,
};
use betterlite_logging::datadog::cost_management::datadog_cost_management::settings::CostManagementInit;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use jiff::tz::TimeZone;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use self::transport::{Reply, ScriptedTransport};

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
    cost_tag_keys: Vec<String>,
    batch_size: Option<usize>,
    flush_interval: Option<f64>,
    max_queue_size: Option<usize>,
}

#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
enum Action {
    Success {
        payload: Box<StandardLoggingPayload>,
    },
    SendBatch,
    Flush,
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

fn event(payload: StandardLoggingPayload) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

fn init(scenario: &Scenario) -> CostManagementInit {
    CostManagementInit {
        cost_tag_keys: scenario.init.cost_tag_keys.clone(),
        batch_size: scenario.init.batch_size,
        flush_interval: scenario
            .init
            .flush_interval
            .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok()),
        max_queue_size: scenario.init.max_queue_size,
    }
}

async fn perform(
    logger: &Arc<DatadogCostManagementLogger<ScriptedTransport>>,
    action: &Action,
) -> Result<Value, HelperFailure> {
    match action {
        Action::Success { payload } => {
            logger
                .async_log_success_event(event(payload.as_ref().clone()))
                .await
                .map_err(value_error)?;
        }
        Action::SendBatch | Action::Flush => {
            logger.flush_queue().await;
        }
    }
    Ok(json!({"queue_length": logger.queue_length()}))
}

fn play(
    scenario: &Scenario,
    logger: &Arc<DatadogCostManagementLogger<ScriptedTransport>>,
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
    Ok(json!({
        "name": scenario.name,
        "init": {
            "upload_url": logger.settings.upload_url,
            "site": logger.settings.site,
            "cost_tag_keys": logger.settings.cost_tag_keys,
            "batch_size": logger.settings.batch.batch_size,
            "flush_interval": logger.settings.batch.flush_interval.as_secs_f64(),
            "max_queue_size": logger.settings.batch.max_queue_size,
            "authenticated": logger.settings.api_key.is_some() && logger.settings.app_key.is_some(),
        },
        "results": results,
        "calls": logger.transport.calls(),
        "queue_length": logger.queue_length(),
    }))
}

fn run_scenario(scenario: &Scenario) -> Result<Value, HelperFailure> {
    let transport = Arc::new(ScriptedTransport::new(&scenario.replies));
    let logger = DatadogCostManagementLogger::new(
        &init(scenario),
        &scenario.environment,
        CostManagementDeps {
            transport: Arc::clone(&transport),
            zone: TimeZone::system(),
            tasks: None,
            shutdown: CancellationToken::new(),
        },
    )
    .map_err(value_error)?;
    play(scenario, &logger)
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
