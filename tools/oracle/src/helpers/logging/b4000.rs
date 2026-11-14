use std::collections::{BTreeMap, VecDeque};
use std::str::from_utf8;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::azure_storage_logging::filename::RandomIds;
use betterlite_logging::azure_storage_logging::logger::{AzureBlobStorageLogger, AzureStorageDeps};
use betterlite_logging::azure_storage_logging::settings::AzureStorageSettings;
use betterlite_logging::azure_storage_logging::token::{
    ChainError, ChainProviderFactory, ChainTokenProvider,
};
use betterlite_logging::azure_storage_logging::transport::{
    StorageRequest, StorageResponse, StorageTransport, StorageTransportError,
};
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::datadog_logging::clock::Clock;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::decisions::Json;
use http::Method;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio_util::sync::CancellationToken;

use crate::helpers::{Helper, HelperResult, OrHelperError};

const START_SECONDS: u64 = 1_700_000_000;
const TOKEN_SUFFIX: &str = "/oauth2/v2.0/token";
const MARKERS: [(&str, &str); 4] = [
    ("resource=directory", "create_directory"),
    ("resource=file", "create_file"),
    ("action=append", "append"),
    ("action=flush", "flush"),
];
const START_DATE: &str = "2023-11-14";

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    scenarios: Vec<Scenario>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Scenario {
    environment: BTreeMap<String, String>,
    payload: Option<Value>,
    replies: Vec<Reply>,
    actions: Vec<Action>,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Reply {
    status: u16,
    body: String,
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
            .build()
            .or_config_error()?;
        let results: Vec<Value> = input
            .scenarios
            .iter()
            .map(|scenario| runtime.block_on(drive(scenario)))
            .collect();
        Ok(json!({ "results": results }))
    }
}

async fn drive(scenario: &Scenario) -> Value {
    let environment = scenario.environment.clone();
    let settings = match AzureStorageSettings::from_environment(&environment) {
        Ok(settings) => settings,
        Err(error) => return json!({ "initialized": false, "error": error.to_string() }),
    };
    let endpoints = json!({
        "dfs": settings.dfs_endpoint(),
        "blob": settings.blob_endpoint(),
    });
    let transport = Arc::new(ScriptedTransport {
        replies: Mutex::new(scenario.replies.iter().cloned().collect()),
        calls: Mutex::new(Vec::new()),
    });
    let clock = Arc::new(ManualClock(Mutex::new(Duration::from_secs(START_SECONDS))));
    let logger = AzureBlobStorageLogger::new(
        settings,
        AzureStorageDeps {
            transport: Arc::<ScriptedTransport>::clone(&transport),
            clock: Arc::<ManualClock>::clone(&clock),
            environment: Arc::new(environment),
            ids: Arc::new(RandomIds),
            chain_factory: Arc::new(FixedFactory),
            shutdown: CancellationToken::new(),
            tasks: None,
        },
    );
    let base = scenario.payload.clone().unwrap_or(Value::Null);
    let mut results = Vec::with_capacity(scenario.actions.len());
    for action in &scenario.actions {
        results.push(perform(action, &logger, &clock, &base).await);
    }
    report(&logger, &transport, endpoints, results)
}

fn report(
    logger: &AzureBlobStorageLogger,
    transport: &ScriptedTransport,
    endpoints: Value,
    results: Vec<Value>,
) -> Value {
    let queue: Vec<String> = logger
        .queue
        .pending()
        .into_iter()
        .map(|record| record.id)
        .collect();
    json!({
        "initialized": true,
        "endpoints": endpoints,
        "actions": results,
        "queue": queue,
        "calls": transport
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(describe)
            .collect::<Vec<Value>>(),
        "clients": {
            "created": logger.created_clients.load(Ordering::SeqCst),
            "closed": logger.closed_clients.load(Ordering::SeqCst),
        },
    })
}

fn payload_for(base: &Value, id: &str) -> Result<StandardLoggingPayload, String> {
    match base.clone() {
        Value::Object(mut fields) => {
            let _replaced = fields.insert("id".to_owned(), Value::String(id.to_owned()));
            serde_json::from_value(Value::Object(fields)).map_err(|error| error.to_string())
        }
        _ => Err("the scenario has no payload".to_owned()),
    }
}

async fn perform(
    action: &Action,
    logger: &AzureBlobStorageLogger,
    clock: &ManualClock,
    base: &Value,
) -> Value {
    match action {
        Action::Event { id, failure } => {
            let Ok(payload) = payload_for(base, id) else {
                return json!({ "raised": true });
            };
            let queued = Arc::new(LoggerEvent {
                payload: Arc::new(payload),
                dynamic_params: Arc::new(Json::Null),
                litellm_metadata: Arc::new(IndexMap::new()),
                stream: false,
                guardrail_snapshot: None,
            });
            let outcome = if *failure {
                logger.async_log_failure_event(queued).await
            } else {
                logger.async_log_success_event(queued).await
            };
            outcome
                .err()
                .map_or(Value::Null, |_| json!({ "raised": true }))
        }
        Action::Upload { id } => {
            let Ok(payload) = payload_for(base, id) else {
                return json!({ "raised": true });
            };
            logger
                .upload_payload(&payload)
                .await
                .err()
                .map_or(Value::Null, |_| json!({ "raised": true }))
        }
        Action::Flush => {
            let _report = logger.flush_queue().await;
            Value::Null
        }
        Action::Advance { seconds } => {
            let mut now = clock.0.lock().unwrap_or_else(PoisonError::into_inner);
            *now = now.saturating_add(Duration::from_secs(*seconds));
            Value::Null
        }
    }
}

struct FixedFactory;
impl ChainProviderFactory for FixedFactory {
    fn provider(&self) -> Result<Arc<dyn ChainTokenProvider>, ChainError> {
        Err(ChainError::Unavailable)
    }
}

struct ManualClock(Mutex<Duration>);
impl Clock for ManualClock {
    fn now_unix(&self) -> Duration {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct ScriptedTransport {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<StorageRequest>>,
}
fn operation(call: &StorageRequest) -> &'static str {
    let url = call.url.as_str();
    if url.ends_with(TOKEN_SUFFIX) {
        return "entra_token";
    }
    if call.method == Method::HEAD {
        return "exists";
    }
    MARKERS
        .iter()
        .find(|pair| url.contains(pair.0))
        .map_or("other", |pair| pair.1)
}

fn describe(call: &StorageRequest) -> Value {
    let op = operation(call);
    if op == "entra_token" {
        return json!({ "op": op });
    }
    let url = call.url.replace(START_DATE, "{date}");
    if op != "append" {
        return json!({ "op": op, "url": url });
    }
    let text = from_utf8(&call.body).unwrap_or_default();
    let body: Option<Value> = serde_json::from_str(text.trim_end_matches('\n')).ok();
    json!({ "op": op, "url": url, "body": body, "trailing_newline": text.ends_with('\n') })
}

#[async_trait]
impl StorageTransport for ScriptedTransport {
    async fn send(
        &self,
        request: StorageRequest,
    ) -> Result<StorageResponse, StorageTransportError> {
        let token_request = request.url.ends_with(TOKEN_SUFFIX);
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        if token_request {
            return Ok(StorageResponse {
                status: 200,
                body: br#"{"access_token": "tok-1"}"#.to_vec(),
            });
        }
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_default();
        Ok(StorageResponse {
            status: reply.status,
            body: reply.body.into_bytes(),
        })
    }
}
