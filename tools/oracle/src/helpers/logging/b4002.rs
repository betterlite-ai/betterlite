use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::azure_sentinel_logging::logger::{AzureSentinelLogger, SentinelDeps};
use betterlite_logging::azure_sentinel_logging::settings::{SentinelInit, SentinelSettings};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperError, HelperResult};

use self::logs::Logs;
use self::scripted::{ManualClock, Reply, ScriptedTransport};

mod logs;
mod records;
mod scripted;

const START_SECONDS: u64 = 1_000_000;
// a request that never answers is a cancelled flush; time is paused, so the wait costs nothing
const ACTION_DEADLINE: Duration = Duration::from_secs(60);

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    scenarios: Vec<Scenario>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Scenario {
    environment: BTreeMap<String, String>,
    init: SentinelInit,
    batch_size: Option<usize>,
    max_queue_size: Option<usize>,
    replies: Vec<Reply>,
    actions: Vec<Action>,
}

#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Model {
        #[serde(default)]
        ids: Vec<String>,
        #[serde(default)]
        failure: bool,
        #[serde(default)]
        pad: usize,
        #[serde(default)]
        missing: bool,
    },
    Audit {
        #[serde(default)]
        ids: Vec<String>,
    },
    Flush,
    Advance {
        seconds: u64,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .map_err(|error| {
                Box::new(HelperError {
                    class: "RuntimeError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
        let results: Vec<Value> = input
            .scenarios
            .iter()
            .map(|scenario| scenario_report(&runtime, scenario))
            .collect();
        Ok(json!({ "results": results }))
    }
}

fn scenario_report(runtime: &Runtime, scenario: &Scenario) -> Value {
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let _guard = tracing::subscriber::set_default(subscriber);
    runtime.block_on(drive(scenario, &logs))
}

fn describe(settings: &SentinelSettings) -> Value {
    json!({
        "model_url": settings.model_url,
        "audit_url": settings.audit_url,
        "token_url": settings.token_url,
        "scope": settings.scope,
        "client_id": settings.client_id,
        "client_secret": settings.client_secret.expose_secret(),
        "batch_size": settings.batch.batch_size,
        "flush_interval": settings.batch.flush_interval.as_secs(),
        "max_queue_size": settings.batch.max_queue_size,
    })
}

async fn drive(scenario: &Scenario, logs: &Logs) -> Value {
    let mut settings = match SentinelSettings::resolve(&scenario.init, &scenario.environment) {
        Ok(settings) => settings,
        Err(error) => return json!({ "initialized": false, "error": error.to_string() }),
    };
    let described = describe(&settings);
    if let Some(size) = scenario.batch_size {
        settings.batch.batch_size = size;
    }
    if let Some(size) = scenario.max_queue_size {
        settings.batch.max_queue_size = size;
    }
    let transport = Arc::new(ScriptedTransport::new(&scenario.replies));
    let clock = Arc::new(ManualClock(Mutex::new(Duration::from_secs(START_SECONDS))));
    let logger = AzureSentinelLogger::new(
        settings,
        SentinelDeps {
            transport: Arc::<ScriptedTransport>::clone(&transport),
            clock: Arc::<ManualClock>::clone(&clock),
            stopped: CancellationToken::new(),
            tasks: None,
        },
    );
    transport.attach(&logger);
    let mut results = Vec::with_capacity(scenario.actions.len());
    for action in &scenario.actions {
        let outcome = tokio::time::timeout(ACTION_DEADLINE, perform(action, &logger, &clock)).await;
        results.push(match outcome {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => json!({ "error": error.to_string() }),
            Err(_elapsed) => json!({ "cancelled": true }),
        });
    }
    report(&logger, &transport, &described, &results, logs)
}

fn report(
    logger: &AzureSentinelLogger,
    transport: &ScriptedTransport,
    settings: &Value,
    actions: &[Value],
    logs: &Logs,
) -> Value {
    let model: Vec<String> = logger
        .model
        .queue
        .pending()
        .into_iter()
        .map(|record| record.id)
        .collect();
    let audit: Vec<String> = logger
        .audit
        .queue
        .pending()
        .into_iter()
        .map(|record| record.id)
        .collect();
    json!({
        "initialized": true,
        "settings": settings,
        "actions": actions,
        "model_queue": model,
        "audit_queue": audit,
        "awaiting": {
            "model": logger.model.awaiting_retry.load(Ordering::SeqCst),
            "audit": logger.audit.awaiting_retry.load(Ordering::SeqCst),
        },
        "calls": transport.calls(),
        "logs": logs.counts(),
    })
}

async fn perform(
    action: &Action,
    logger: &AzureSentinelLogger,
    clock: &ManualClock,
) -> Result<Value, serde_json::Error> {
    match action {
        Action::Model {
            ids,
            failure,
            pad,
            missing,
        } => {
            if *missing {
                logger.async_log_model_event(None).await;
            }
            for id in ids {
                let payload = records::model_payload(id, *failure, *pad)?;
                logger.async_log_model_event(Some(&payload)).await;
            }
            Ok(Value::Null)
        }
        Action::Audit { ids } => {
            for id in ids {
                logger
                    .async_log_audit_log_event(records::audit_payload(id))
                    .await;
            }
            Ok(Value::Null)
        }
        Action::Flush => {
            logger.flush_queue().await;
            let flushed = *logger
                .last_flush_time
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            Ok(json!({ "last_flush_time": flushed.as_secs() }))
        }
        Action::Advance { seconds } => {
            advance(clock, *seconds);
            Ok(Value::Null)
        }
    }
}

fn advance(clock: &ManualClock, seconds: u64) {
    let mut now = clock.0.lock().unwrap_or_else(PoisonError::into_inner);
    let later = now.saturating_add(Duration::from_secs(seconds));
    *now = later;
}
