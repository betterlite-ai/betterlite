use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::azure_sentinel_logging::logger::{AzureSentinelLogger, SentinelDeps};
use betterlite_logging::azure_sentinel_logging::settings::{SentinelInit, SentinelSettings};
use secrecy::{ExposeSecret as _, SecretString};
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    init: Init,
    #[serde(default)]
    batch_size: Option<usize>,
    #[serde(default)]
    max_queue_size: Option<usize>,
    #[serde(default)]
    replies: Vec<Reply>,
    #[serde(default)]
    actions: Vec<Action>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Init {
    dcr_immutable_id: Option<String>,
    stream_name: Option<String>,
    endpoint: Option<String>,
    tenant_id: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    audit_stream_name: Option<String>,
    authority_host: Option<String>,
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

fn sentinel_init(init: &Init) -> SentinelInit {
    SentinelInit {
        dcr_immutable_id: init.dcr_immutable_id.clone(),
        stream_name: init.stream_name.clone(),
        endpoint: init.endpoint.clone(),
        tenant_id: init.tenant_id.clone(),
        client_id: init.client_id.clone(),
        client_secret: init.client_secret.clone().map(SecretString::from),
        audit_stream_name: init.audit_stream_name.clone(),
        authority_host: init.authority_host.clone(),
    }
}

async fn drive(scenario: &Scenario, logs: &Logs) -> Value {
    let init = sentinel_init(&scenario.init);
    let mut settings = match SentinelSettings::resolve(&init, &scenario.environment) {
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
    let wire = Arc::clone(&transport);
    let ticks = Arc::clone(&clock);
    let logger = AzureSentinelLogger::new(
        settings,
        SentinelDeps {
            transport: wire,
            clock: ticks,
            stopped: CancellationToken::new(),
            tasks: None,
        },
    );
    transport.attach(&logger);
    let results = run_actions(&scenario.actions, &logger, &clock).await;
    report(&logger, &transport, &described, &results, logs)
}

async fn run_actions(
    actions: &[Action],
    logger: &AzureSentinelLogger,
    clock: &ManualClock,
) -> Vec<Value> {
    let mut results = Vec::with_capacity(actions.len());
    for action in actions {
        let outcome = tokio::time::timeout(ACTION_DEADLINE, perform(action, logger, clock)).await;
        results.push(match outcome {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => json!({ "error": error.to_string() }),
            Err(_elapsed) => json!({ "cancelled": true }),
        });
    }
    results
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
            } else {
                log_models(logger, ids, *failure, *pad).await?;
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
            let mut now = clock.0.lock().unwrap_or_else(PoisonError::into_inner);
            let later = now.saturating_add(Duration::from_secs(*seconds));
            *now = later;
            Ok(Value::Null)
        }
    }
}

async fn log_models(
    logger: &AzureSentinelLogger,
    ids: &[String],
    failure: bool,
    pad: usize,
) -> Result<(), serde_json::Error> {
    for id in ids {
        let payload = records::model_payload(id, failure, pad)?;
        logger.async_log_model_event(Some(&payload)).await;
    }
    Ok(())
}
