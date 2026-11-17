use std::collections::{BTreeMap, VecDeque};
use std::io::{Result as IoResult, Write};
use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::logging_failure::events::payload_to_map;
use crate::zerobus::client::init_params::ZerobusInitParams;
use crate::zerobus::client::types::{ZerobusConnection, ZerobusIngestFailure};
use crate::zerobus::logger::connection::SecretSource;
use crate::zerobus::logger::delivery::RowIngest;
use crate::zerobus::logger::payload::RedactionSettings;
use crate::zerobus::logger::{ZerobusDeps, ZerobusLogger};
use crate::zerobus::row::{TraceRow, trace_row};

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.0,
    "completionStartTime": 1704067200.5,
    "response_time": 0.5,
    "model_map_information": {},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "end_user": "user-1",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "hi"}],
    "response": {"choices": [{"message": {"content": "the secret answer"}}]}
}"#;

pub(super) fn payload(request_id: &str) -> StandardLoggingPayload {
    let mut payload: StandardLoggingPayload =
        serde_json::from_str(PAYLOAD).expect("a valid standard logging payload");
    payload.id = request_id.to_owned();
    payload
}

pub(super) fn event(request_id: &str) -> Arc<LoggerEvent> {
    event_with(
        payload(request_id),
        Json::Object(IndexMap::new()),
        IndexMap::new(),
    )
}

pub(super) fn event_with(
    payload: StandardLoggingPayload,
    dynamic_params: Json,
    litellm_metadata: IndexMap<String, Json>,
) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(dynamic_params),
        litellm_metadata: Arc::new(litellm_metadata),
        stream: false,
        guardrail_snapshot: None,
    })
}

pub(super) fn params(
    batch_size: usize,
    flush_interval: u64,
    turn_off_message_logging: bool,
) -> ZerobusInitParams {
    ZerobusInitParams {
        workspace_url: None,
        server_endpoint: None,
        client_id: None,
        client_secret: None,
        table_name: None,
        batch_size: NonZeroUsize::new(batch_size).expect("the batch size is nonzero"),
        flush_interval: NonZeroU64::new(flush_interval).expect("the flush interval is nonzero"),
        turn_off_message_logging,
    }
}

pub(super) fn connection(table_name: &str) -> ZerobusConnection {
    let secrets = connection_secrets();
    let lookup = |name: &str| {
        secrets
            .get_secret_str(name)
            .expect("the fixture carries the connection settings")
    };
    ZerobusConnection {
        workspace_url: lookup("ZEROBUS_WORKSPACE_URL").expose_secret().to_owned(),
        workspace_id: "1234567890123456".to_owned(),
        server_endpoint: lookup("ZEROBUS_SERVER_ENDPOINT").expose_secret().to_owned(),
        client_id: lookup("ZEROBUS_CLIENT_ID").expose_secret().to_owned(),
        client_secret: lookup("ZEROBUS_CLIENT_SECRET"),
        table_name: table_name.to_owned(),
    }
}

pub(super) fn connection_secrets() -> MapSecrets {
    MapSecrets::new(&[
        (
            "ZEROBUS_SERVER_ENDPOINT",
            "https://1234567890123456.zerobus.us-west-2.cloud.databricks.com",
        ),
        (
            "ZEROBUS_WORKSPACE_URL",
            "https://dbc-1a2b3c4d.cloud.databricks.com",
        ),
        ("ZEROBUS_TABLE_NAME", "main.litellm.traces"),
        ("ZEROBUS_CLIENT_ID", "sp-client-id"),
        ("ZEROBUS_CLIENT_SECRET", "sp-client-secret"),
    ])
}

pub(super) fn proxied_client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    ProxiedClient::outbound(&policy).expect("the HTTP client builds")
}

pub(super) fn deps(params: ZerobusInitParams, client: Option<Arc<dyn RowIngest>>) -> ZerobusDeps {
    ZerobusDeps {
        params,
        client,
        secrets: Arc::new(MapSecrets::new(&[])),
        http: proxied_client(),
        clock: Arc::new(|| 0.0_f64),
        redaction: RedactionSettings {
            turn_off_message_logging: false,
            excluded_fields: Vec::new(),
        },
        tasks: TaskTracker::new(),
        shutdown: CancellationToken::new(),
        start_periodic_flush: false,
        max_queue_size: None,
    }
}

pub(super) fn logger(params: ZerobusInitParams, client: Arc<dyn RowIngest>) -> ZerobusLogger {
    ZerobusLogger::new(deps(params, Some(client))).expect("the logger builds")
}

pub(super) fn as_row_ingest(ingest: &Arc<FakeIngest>) -> Arc<dyn RowIngest> {
    let clone = Arc::clone(ingest);
    let shared: Arc<dyn RowIngest> = clone;
    shared
}

pub(super) fn as_secret_source(secrets: &Arc<MapSecrets>) -> Arc<dyn SecretSource> {
    let clone = Arc::clone(secrets);
    let shared: Arc<dyn SecretSource> = clone;
    shared
}

pub(super) fn row(request_id: &str) -> TraceRow {
    let map = payload_to_map(&payload(request_id)).expect("the payload maps to JSON");
    trace_row(&map).expect("the payload builds a row")
}

pub(super) fn push(logger: &ZerobusLogger, request_id: &str) -> usize {
    logger.inner.queue.enqueue(row(request_id)).depth
}

pub(super) async fn log_event(logger: &ZerobusLogger, request_id: &str) {
    logger
        .async_log_success_event(event(request_id))
        .await
        .expect("the event is accepted");
}

pub(super) async fn until<F>(check: F) -> bool
where
    F: FnMut() -> bool,
{
    until_within(200, check).await
}

pub(super) async fn until_within<F>(attempts: usize, mut check: F) -> bool
where
    F: FnMut() -> bool,
{
    for _ in 0..attempts {
        if check() {
            return true;
        }
        sleep(Duration::from_millis(1)).await;
    }
    false
}

pub(super) fn task_alive(handle: &Mutex<Option<JoinHandle<()>>>) -> bool {
    let stored = handle.lock().unwrap_or_else(PoisonError::into_inner);
    stored.as_ref().is_some_and(|task| !task.is_finished())
}

pub(super) async fn settle(logger: &ZerobusLogger) {
    let idle =
        until(|| !task_alive(&logger.batch) && !logger.inner.flushing.load(Ordering::SeqCst)).await;
    assert!(idle, "the flush did not settle");
}

pub(super) fn queued_rows(logger: &ZerobusLogger) -> Vec<TraceRow> {
    logger.inner.queue.pending()
}

pub(super) fn queued_ids(logger: &ZerobusLogger) -> Vec<Option<String>> {
    queued_rows(logger).into_iter().map(|row| row.id).collect()
}

pub(super) fn expected_ids(names: &[&str]) -> Vec<Option<String>> {
    names.iter().map(|name| Some((*name).to_owned())).collect()
}

#[derive(Clone, Default)]
pub(super) struct MapSecrets {
    values: Arc<Mutex<BTreeMap<String, String>>>,
}

impl MapSecrets {
    pub(super) fn new(values: &[(&str, &str)]) -> Self {
        let stored: BTreeMap<String, String> = values
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        Self {
            values: Arc::new(Mutex::new(stored)),
        }
    }

    pub(super) fn set(&self, name: &str, value: &str) {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), value.to_owned());
    }
}

impl SecretSource for MapSecrets {
    fn get_secret_str(&self, name: &str) -> Option<SecretString> {
        let variable = name.strip_prefix("os.environ/").unwrap_or(name);
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(variable)
            .map(|value| SecretString::from(value.as_str()))
    }
}

pub(super) struct FakeIngest {
    connection: ZerobusConnection,
    batches: Mutex<Vec<Vec<TraceRow>>>,
    outcomes: Mutex<VecDeque<Result<(), ZerobusIngestFailure>>>,
    gate: Mutex<Option<Arc<Semaphore>>>,
}

impl FakeIngest {
    pub(super) fn new(connection: ZerobusConnection) -> Self {
        Self {
            connection,
            batches: Mutex::new(Vec::new()),
            outcomes: Mutex::new(VecDeque::new()),
            gate: Mutex::new(None),
        }
    }

    pub(super) fn table(table_name: &str) -> Arc<Self> {
        Arc::new(Self::new(connection(table_name)))
    }

    pub(super) fn failing(failure: ZerobusIngestFailure) -> Arc<Self> {
        let ingest = Self::new(connection("main.litellm.traces"));
        ingest
            .outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Err(failure));
        Arc::new(ingest)
    }

    pub(super) fn block(&self) -> Arc<Semaphore> {
        let gate = Arc::new(Semaphore::new(0));
        *self.gate.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&gate));
        gate
    }

    pub(super) fn unblock(&self) {
        *self.gate.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    pub(super) fn batches(&self) -> Vec<Vec<TraceRow>> {
        self.batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn inserted(&self) -> usize {
        self.batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(super) fn ids(&self) -> Vec<Option<String>> {
        self.batches()
            .into_iter()
            .flatten()
            .map(|row| row.id)
            .collect()
    }
}

#[async_trait]
impl RowIngest for FakeIngest {
    fn connection(&self) -> &ZerobusConnection {
        &self.connection
    }

    async fn ingest(&self, rows: &[TraceRow]) -> Result<(), ZerobusIngestFailure> {
        self.batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(rows.to_vec());
        let held = self
            .gate
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(gate) = held
            && let Ok(permit) = gate.acquire().await
        {
            permit.forget();
        }
        self.outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(()))
    }
}

#[derive(Clone, Default)]
pub(super) struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    pub(super) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).expect("the log is UTF-8")
    }
}

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Buffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn subscribe(buffer: &Buffer) -> DefaultGuard {
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    set_default(subscriber)
}
