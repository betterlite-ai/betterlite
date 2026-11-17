use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::io::{Result as IoResult, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::pointfive_params::{
    DEFAULT_BATCH_SIZE, DEFAULT_FLUSH_INTERVAL, DEFAULT_MAX_BATCH_BYTES,
    DEFAULT_MAX_UPLOAD_RETRIES, PointFiveInitParams,
};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::user_url::error::UserUrlError;
use betterlite_models::decisions::Json;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use http::Method;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::task::JoinHandle;
use tokio::task::yield_now;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::MakeWriter;

use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::finops::pointfive::logger::client::{PointFiveService, UploadClientSource};
use crate::finops::pointfive::logger::core::PointFiveCore;
use crate::finops::pointfive::logger::error::PointFiveError;
use crate::finops::pointfive::logger::{PointFiveDeps, PointFiveLogger};
use crate::finops::pointfive::upload::PointFiveUploadClient;
use crate::finops::pointfive::upload::sleeper::UploadSleeper;
use crate::finops::pointfive::upload::transport::{UploadHttp, UploadRequest, UploadResponse};
use crate::finops::pointfive::upload::url_check::{UploadUrlCheck, ValidatedUpload};
use crate::sqs_logging::health::HEALTH_CHECK_PAYLOAD;

pub(super) const API_URL: &str = "https://api.pointfive.test/ingestion";
pub(super) const PRESIGNED: &str = "https://uploads.example/object";

#[derive(Debug, Clone)]
pub(super) enum Reply {
    Answer { status: u16, body: Vec<u8> },
    Transport,
}

impl Reply {
    pub(super) fn answer(status: u16) -> Self {
        Self::Answer {
            status,
            body: Vec::new(),
        }
    }

    pub(super) fn with_body(status: u16, body: &str) -> Self {
        Self::Answer {
            status,
            body: body.as_bytes().to_vec(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seen {
    pub(super) method: Method,
    pub(super) url: String,
    pub(super) body: Vec<u8>,
}

#[derive(Clone, Default)]
pub(super) struct FakeHttp {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Seen>>>,
}

impl FakeHttp {
    pub(super) fn scripted(replies: Vec<Reply>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(VecDeque::from(replies))),
            requests: Arc::default(),
        }
    }

    pub(super) fn uploading(uploads: usize) -> Self {
        let mut replies = Vec::new();
        for index in 0..uploads {
            let presigned = format!("{PRESIGNED}/{index}");
            replies.push(Reply::with_body(
                200,
                &target_body(&presigned, &format!("objects/{index}")),
            ));
            replies.push(Reply::answer(200));
        }
        Self::scripted(replies)
    }

    pub(super) fn requests(&self) -> Vec<Seen> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn uploads(&self) -> Vec<Seen> {
        self.requests()
            .into_iter()
            .filter(|seen| seen.method == Method::PUT)
            .collect()
    }

    pub(super) fn pings(&self) -> Vec<Seen> {
        self.requests()
            .into_iter()
            .filter(|seen| seen.url.ends_with("/ping"))
            .collect()
    }
}

#[async_trait]
impl UploadHttp for FakeHttp {
    async fn send(&self, request: UploadRequest) -> Result<UploadResponse, TransportError> {
        let url = request.url.to_string();
        let is_ping = url.ends_with("/ping");
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Seen {
                method: request.method,
                url,
                body: request.body,
            });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match reply {
            Some(Reply::Answer { status, body }) => Ok(UploadResponse { status, body }),
            Some(Reply::Transport) => Err(TransportError::BodyTooLarge { limit: 1 }),
            // a background liveness ping may fire at any time, so one that was not scripted is fine
            None if is_ping => Ok(UploadResponse {
                status: 200,
                body: Vec::new(),
            }),
            None => panic!("the test scripted no reply"),
        }
    }
}

pub(super) struct FakeUrlCheck;

#[async_trait]
impl UploadUrlCheck for FakeUrlCheck {
    async fn check(&self, url: &str) -> Result<ValidatedUpload, UserUrlError> {
        Ok(ValidatedUpload {
            url: url.to_owned(),
            host_header: None,
        })
    }
}

#[derive(Clone, Default)]
pub(super) struct RecordingSleeper {
    pub(super) delays: Arc<Mutex<Vec<Duration>>>,
}

#[async_trait]
impl UploadSleeper for RecordingSleeper {
    async fn sleep(&self, delay: Duration) {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(delay);
    }
}

pub(super) struct FixedSource {
    pub(super) client: PointFiveUploadClient,
    pub(super) calls: Arc<AtomicUsize>,
}

impl UploadClientSource for FixedSource {
    fn upload_client(
        &self,
        _params: &PointFiveInitParams,
    ) -> Result<PointFiveUploadClient, PointFiveError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.client.clone())
    }
}

pub(super) struct MissingKeySource;

impl UploadClientSource for MissingKeySource {
    fn upload_client(
        &self,
        _params: &PointFiveInitParams,
    ) -> Result<PointFiveUploadClient, PointFiveError> {
        Err(PointFiveError::MissingApiKey)
    }
}

pub(super) struct LapsingSource {
    pub(super) client: PointFiveUploadClient,
    pub(super) available: Arc<AtomicBool>,
    pub(super) lapse: Lapse,
}

#[derive(Clone, Copy)]
pub(super) enum Lapse {
    MissingKey,
    Secret,
}

impl UploadClientSource for LapsingSource {
    fn upload_client(
        &self,
        _params: &PointFiveInitParams,
    ) -> Result<PointFiveUploadClient, PointFiveError> {
        if self.available.load(Ordering::SeqCst) {
            return Ok(self.client.clone());
        }
        match self.lapse {
            Lapse::MissingKey => Err(PointFiveError::MissingApiKey),
            Lapse::Secret => Err(PointFiveError::Secret(SecretLookupError::GoogleOidcFailed)),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct MutableEnvironment(pub(super) Arc<Mutex<BTreeMap<String, String>>>);

impl MutableEnvironment {
    pub(super) fn with(pairs: &[(&str, &str)]) -> Self {
        let environment = Self::default();
        for (name, value) in pairs {
            environment.set(name, value);
        }
        environment
    }

    pub(super) fn set(&self, name: &str, value: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), value.to_owned());
    }

    pub(super) fn unset(&self, name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name);
    }
}

impl EnvironmentLookup for MutableEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
    }
}

pub(super) fn target_body(upload_url: &str, object_key: &str) -> String {
    format!(r#"{{"uploadUrl":"{upload_url}","objectKey":"{object_key}"}}"#)
}

pub(super) fn default_params() -> PointFiveInitParams {
    PointFiveInitParams {
        api_key: None,
        api_url: None,
        batch_size: DEFAULT_BATCH_SIZE,
        flush_interval: DEFAULT_FLUSH_INTERVAL,
        max_batch_bytes: DEFAULT_MAX_BATCH_BYTES,
        max_upload_retries: DEFAULT_MAX_UPLOAD_RETRIES,
        turn_off_message_logging: false,
    }
}

pub(super) fn batching(batch_size: usize) -> PointFiveInitParams {
    let mut params = default_params();
    params.batch_size = batch_size;
    params
}

pub(super) fn client_over(
    http: Arc<dyn UploadHttp>,
    url_check: Arc<dyn UploadUrlCheck>,
    sleeper: Arc<dyn UploadSleeper>,
    max_retries: usize,
) -> PointFiveUploadClient {
    PointFiveUploadClient {
        api_key: SecretString::from("key-1"),
        api_url: API_URL.to_owned(),
        http,
        url_check,
        sleeper,
        max_retries,
        request_timeout: Duration::from_secs(5),
    }
}

pub(super) fn client_with(
    http: &FakeHttp,
    max_retries: usize,
    sleeper: RecordingSleeper,
) -> PointFiveUploadClient {
    client_over(
        Arc::new(http.clone()),
        Arc::new(FakeUrlCheck),
        Arc::new(sleeper),
        max_retries,
    )
}

pub(super) fn client(http: &FakeHttp, max_retries: usize) -> PointFiveUploadClient {
    client_with(http, max_retries, RecordingSleeper::default())
}

pub(super) fn service(
    http: &FakeHttp,
    environment: Arc<dyn EnvironmentLookup + Send + Sync>,
) -> PointFiveService {
    let mut service =
        PointFiveService::new(Arc::new(http.clone()), Arc::new(FakeUrlCheck), environment);
    service.sleeper = Arc::new(RecordingSleeper::default());
    service.request_timeout = Duration::from_secs(5);
    service
}

pub(super) fn core_with(
    params: PointFiveInitParams,
    client: PointFiveUploadClient,
    global_message_redaction: bool,
) -> Arc<PointFiveCore> {
    Arc::new(PointFiveCore::new(
        params,
        Some(client.clone()),
        Arc::new(FixedSource {
            client,
            calls: Arc::default(),
        }),
        global_message_redaction,
    ))
}

pub(super) fn core(
    params: PointFiveInitParams,
    client: PointFiveUploadClient,
) -> Arc<PointFiveCore> {
    core_with(params, client, false)
}

pub(super) fn deps_from(
    source: Arc<dyn UploadClientSource>,
    given_upload_client: Option<PointFiveUploadClient>,
    start_periodic_flush: bool,
) -> PointFiveDeps {
    PointFiveDeps {
        source,
        given_upload_client,
        global_message_redaction: false,
        start_periodic_flush,
        tasks: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

pub(super) fn deps(client: PointFiveUploadClient) -> PointFiveDeps {
    let source = Arc::new(FixedSource {
        client: client.clone(),
        calls: Arc::default(),
    });
    deps_from(source, Some(client), false)
}

pub(super) fn logger(
    params: PointFiveInitParams,
    client: PointFiveUploadClient,
) -> Arc<PointFiveLogger> {
    PointFiveLogger::new(params, deps(client)).expect("the logger builds")
}

pub(super) fn logger_started(
    params: PointFiveInitParams,
    client: PointFiveUploadClient,
) -> Arc<PointFiveLogger> {
    let source = Arc::new(FixedSource {
        client: client.clone(),
        calls: Arc::default(),
    });
    PointFiveLogger::new(params, deps_from(source, Some(client), true)).expect("the logger builds")
}

pub(super) fn take_periodic_task(logger: &PointFiveLogger) -> JoinHandle<()> {
    logger
        .periodic_task
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .expect("the periodic flush was started")
}

pub(super) fn payload(id: &str) -> StandardLoggingPayload {
    let mut payload: StandardLoggingPayload =
        serde_json::from_str(HEALTH_CHECK_PAYLOAD).expect("the dummy payload parses");
    id.clone_into(&mut payload.id);
    payload
}

pub(super) fn fields(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) fn payload_with_messages(id: &str) -> StandardLoggingPayload {
    let mut payload = payload(id);
    let message = fields(&[
        ("role", Json::Text("user".to_owned())),
        ("content", Json::Text("the secret prompt".to_owned())),
    ]);
    let choice = fields(&[(
        "message",
        Json::Object(fields(&[(
            "content",
            Json::Text("the secret answer".to_owned()),
        )])),
    )]);
    let response = fields(&[
        ("id", Json::Text("chatcmpl-1".to_owned())),
        (
            "created",
            Json::Number(serde_json::Number::from(1_700_000_000_u64)),
        ),
        ("choices", Json::List(vec![Json::Object(choice)])),
    ]);
    payload.messages = Some(Json::List(vec![Json::Object(message)]));
    payload.response = Some(Json::Object(response));
    payload
}

pub(super) fn event(payload: StandardLoggingPayload) -> Arc<LoggerEvent> {
    event_with(payload, &[], &[])
}

pub(super) fn event_with(
    payload: StandardLoggingPayload,
    metadata: &[(&str, Json)],
    dynamic: &[(&str, Json)],
) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(fields(dynamic))),
        litellm_metadata: Arc::new(fields(metadata)),
        stream: false,
        guardrail_snapshot: None,
    })
}

#[derive(Clone, Default)]
pub(super) struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    pub(super) fn contents(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("the log is UTF-8")
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

pub(super) async fn wait_until<Condition>(mut condition: Condition)
where
    Condition: FnMut() -> bool,
{
    let waited = timeout(Duration::from_secs(5), async {
        while !condition() {
            yield_now().await;
        }
    })
    .await;
    waited.expect("the background task made progress");
}

pub(super) async fn logged<Action>(action: Action) -> (Action::Output, String)
where
    Action: Future,
{
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let guard = set_default(subscriber);
    let output = action.await;
    drop(guard);
    (output, buffer.contents())
}
