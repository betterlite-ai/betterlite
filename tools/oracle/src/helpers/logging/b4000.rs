use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::azure_storage_logging::logger::{AzureBlobStorageLogger, AzureStorageDeps};
use betterlite_logging::azure_storage_logging::settings::AzureStorageSettings;
use betterlite_logging::azure_storage_logging::transport::StorageTransport;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::datadog_logging::clock::Clock;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::decisions::Json;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio_util::sync::CancellationToken;

use crate::helpers::{Helper, HelperError, HelperResult};

use self::support::{FixedChain, FixedFactory, FixedIds, ManualClock, ScriptedTransport};

mod support;

pub(super) const START_SECONDS: u64 = 1_700_000_000;
pub(super) const TOKEN_SUFFIX: &str = "/oauth2/v2.0/token";
pub(super) const CHAIN_TOKEN: &str = "chain-token";

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
    payload: Option<Value>,
    token: Option<String>,
    replies: Vec<Reply>,
    actions: Vec<Action>,
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Reply {
    status: u16,
    body: String,
}

impl Default for Reply {
    fn default() -> Self {
        Self {
            status: 200,
            body: String::new(),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Event {
        id: String,
        #[serde(default)]
        failure: bool,
    },
    Upload {
        id: String,
    },
    Token,
    Flush,
    SendBatch,
    Advance {
        seconds: u64,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|error| Box::new(runtime_error(error.to_string())))?;
        let results: Vec<Value> = input
            .scenarios
            .iter()
            .map(|scenario| runtime.block_on(drive(scenario)))
            .collect();
        Ok(json!({ "results": results }))
    }
}

fn runtime_error(message: String) -> HelperError {
    HelperError {
        class: "RuntimeError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    }
}

async fn drive(scenario: &Scenario) -> Value {
    let environment = scenario.environment.clone();
    let settings = match AzureStorageSettings::from_environment(&environment) {
        Ok(settings) => settings,
        Err(error) => return json!({ "initialized": false, "error": error.to_string() }),
    };
    let described = describe_settings(&settings);
    let transport = Arc::new(ScriptedTransport::new(
        &scenario.replies,
        scenario.token.clone(),
    ));
    let clock = Arc::new(ManualClock(Mutex::new(Duration::from_secs(START_SECONDS))));
    let chain = Arc::new(FixedChain::default());
    let factory = Arc::new(FixedFactory {
        provider: Arc::clone(&chain),
        built: Mutex::new(0),
    });
    let logger = AzureBlobStorageLogger::new(
        settings,
        AzureStorageDeps {
            transport: Arc::clone(&transport) as Arc<dyn StorageTransport>,
            clock: Arc::clone(&clock) as Arc<dyn Clock>,
            environment: Arc::new(environment.clone()),
            ids: Arc::new(FixedIds),
            chain_factory: factory.clone(),
            shutdown: CancellationToken::new(),
            tasks: None,
        },
    );
    let base = scenario.payload.clone().unwrap_or(Value::Null);
    let mut results = Vec::with_capacity(scenario.actions.len());
    for action in &scenario.actions {
        results.push(perform(action, &logger, &clock, &base).await);
    }
    let queue: Vec<String> = logger
        .queue
        .pending()
        .into_iter()
        .map(|record| record.id)
        .collect();
    json!({
        "initialized": true,
        "settings": described,
        "actions": results,
        "queue": queue,
        "calls": transport.described(uses_account_key(&scenario.environment)),
        "clients": {
            "created": logger.created_clients.load(Ordering::SeqCst),
            "closed": logger.closed_clients.load(Ordering::SeqCst),
        },
        "tokens": {
            "factory": *factory.built.lock().unwrap_or_else(PoisonError::into_inner),
            "provider": *chain.calls.lock().unwrap_or_else(PoisonError::into_inner),
        },
    })
}

fn uses_account_key(environment: &BTreeMap<String, String>) -> bool {
    environment
        .get("AZURE_STORAGE_ACCOUNT_KEY")
        .is_some_and(|value| !value.is_empty())
}

fn describe_settings(settings: &AzureStorageSettings) -> Value {
    json!({
        "dfs_endpoint": settings.dfs_endpoint(),
        "blob_endpoint": settings.blob_endpoint(),
        "account_name": settings.account_name.clone(),
        "file_system": settings.file_system.clone(),
        "endpoint_suffix": settings.endpoint_suffix.clone(),
        "tenant_id": settings.tenant_id.clone(),
        "client_id": settings.client_id.clone(),
        "has_client_secret": settings.client_secret.is_some(),
        "has_account_key": settings.account_key.is_some(),
        "batch_size": settings.batch.batch_size,
        "flush_interval": settings.batch.flush_interval.as_secs(),
        "max_queue_size": settings.batch.max_queue_size,
    })
}

fn payload_for(base: &Value, id: &str) -> Result<StandardLoggingPayload, String> {
    let mut value = base.clone();
    match &mut value {
        Value::Object(fields) => {
            let _replaced = fields.insert("id".to_owned(), Value::String(id.to_owned()));
        }
        _ => return Err("the scenario has no payload".to_owned()),
    }
    serde_json::from_value(value).map_err(|error| error.to_string())
}

fn event(base: &Value, id: &str) -> Result<Arc<LoggerEvent>, String> {
    let payload = payload_for(base, id)?;
    Ok(Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Null),
        stream: false,
        guardrail_snapshot: None,
    }))
}

async fn perform(
    action: &Action,
    logger: &AzureBlobStorageLogger,
    clock: &ManualClock,
    base: &Value,
) -> Value {
    match action {
        Action::Event { id, failure } => {
            let Ok(queued) = event(base, id) else {
                return json!({ "raised": "payload" });
            };
            let outcome = if *failure {
                logger.async_log_failure_event(queued).await
            } else {
                logger.async_log_success_event(queued).await
            };
            raised(outcome.err().map(|error| error.to_string()))
        }
        Action::Upload { id } => {
            let Ok(payload) = payload_for(base, id) else {
                return json!({ "raised": "payload" });
            };
            let outcome = logger
                .async_upload_payload_to_azure_blob_storage(&payload)
                .await;
            raised(outcome.err().map(|error| error.to_string()))
        }
        Action::Token => match logger.set_valid_azure_ad_token().await {
            Ok(token) => json!({ "token": token.expose_secret() }),
            Err(error) => json!({ "raised": error.to_string() }),
        },
        Action::Flush => {
            let _report = logger.flush_queue().await;
            Value::Null
        }
        Action::SendBatch => {
            let _uploaded = logger.async_send_batch().await;
            Value::Null
        }
        Action::Advance { seconds } => {
            let mut now = clock.0.lock().unwrap_or_else(PoisonError::into_inner);
            *now = now.saturating_add(Duration::from_secs(*seconds));
            Value::Null
        }
    }
}

fn raised(message: Option<String>) -> Value {
    message.map_or(Value::Null, |text| json!({ "raised": text }))
}
