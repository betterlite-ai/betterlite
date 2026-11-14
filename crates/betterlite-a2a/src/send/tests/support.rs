use std::collections::BTreeMap;
use std::future::Future;
use std::net::TcpListener as StdTcpListener;
use std::num::NonZeroUsize;
use std::path::Path;
use std::str::from_utf8;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap as HttpHeaders, Uri};
use axum::response::{IntoResponse as _, Response};
use axum::routing::{get, post};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::huggingface_hub::{
    OfflineSetting, Settings as HubSettings, TransferSetting,
};
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_core::agents::json::AgentJson;
use betterlite_http::hub_oauth::clock::{HubClock, SystemHubClock};
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::settings::Settings as CounterSettings;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::fetch::TiktokenFetcher;
use betterlite_tokenizer::engine::hub::HubDownloader;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService,
};
use betterlite_tokenizer::engine::selection::Selector;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::client::records::compat::SendMessageRequest;
use crate::completion_bridge::tests::support::litellm_params;
use crate::send::deps::{NativeDeps, StreamingDeps};
use crate::streaming_iterator::{
    DEFAULT_MAX_RETAINED_BYTES, StreamSuccessCall, StreamSuccessDispatch,
};

pub(crate) const CARD_PATH: &str = "/.well-known/agent-card.json";
pub(crate) const CUSTOM_CARD_PATH: &str = "/custom/card.json";

#[derive(Clone)]
pub(crate) struct Seen {
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: String,
}

#[derive(Clone)]
pub(crate) enum Reply {
    Json(String),
    Sse(Vec<String>),
}

#[derive(Clone)]
struct AgentState {
    card: String,
    reply: Reply,
    seen: Arc<Mutex<Vec<Seen>>>,
}

fn router(state: AgentState) -> Router {
    Router::new()
        .route(CARD_PATH, get(card))
        .route(CUSTOM_CARD_PATH, get(card))
        .fallback(post(message))
        .with_state(state)
}

fn record(state: &AgentState, path: &str, headers: &HttpHeaders, body: String) {
    let mut seen_headers = BTreeMap::new();
    for (name, value) in headers {
        if let Ok(text) = value.to_str() {
            seen_headers.insert(name.as_str().to_owned(), text.to_owned());
        }
    }
    state
        .seen
        .lock()
        .expect("the fixture lock is not poisoned")
        .push(Seen {
            path: path.to_owned(),
            headers: seen_headers,
            body,
        });
}

async fn card(State(state): State<AgentState>, headers: HttpHeaders, uri: Uri) -> Response {
    record(&state, uri.path(), &headers, String::new());
    ([(CONTENT_TYPE, "application/json")], state.card).into_response()
}

async fn message(
    State(state): State<AgentState>,
    headers: HttpHeaders,
    uri: Uri,
    body: Bytes,
) -> Response {
    let text = from_utf8(&body).unwrap_or_default();
    record(&state, uri.path(), &headers, text.to_owned());
    match state.reply {
        Reply::Json(answer) => ([(CONTENT_TYPE, "application/json")], answer).into_response(),
        Reply::Sse(frames) => {
            ([(CONTENT_TYPE, "text/event-stream")], frames.concat()).into_response()
        }
    }
}

// Runs the scenario against a live agent that answers the card and every message
pub(crate) async fn with_agent<T, F, Fut>(reply: Reply, scenario: F) -> (T, Vec<Seen>)
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = T>,
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the test listener binds");
    let base = format!(
        "http://{}",
        listener.local_addr().expect("the listener has an address")
    );
    let card = card_json(&base);
    serve_agent(listener, base, card, reply, scenario).await
}

// The same live agent, whose card advertises a released port, as a card can point at another URL
pub(crate) async fn with_agent_advertising_a_dead_port<T, F, Fut>(
    reply: Reply,
    scenario: F,
) -> (T, Vec<Seen>)
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = T>,
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the test listener binds");
    let base = format!(
        "http://{}",
        listener.local_addr().expect("the listener has an address")
    );
    let card = card_json(&unused_base());
    serve_agent(listener, base, card, reply, scenario).await
}

async fn serve_agent<T, F, Fut>(
    listener: TcpListener,
    base: String,
    card: String,
    reply: Reply,
    scenario: F,
) -> (T, Vec<Seen>)
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = T>,
{
    let state = AgentState {
        card,
        reply,
        seen: Arc::default(),
    };
    let seen = Arc::clone(&state.seen);
    let (stop, stopped) = oneshot::channel::<()>();
    let server = axum::serve(listener, router(state)).with_graceful_shutdown(async move {
        stopped.await.unwrap_or_default();
    });
    let run = async move {
        let outcome = scenario(base).await;
        stop.send(()).unwrap_or_default();
        outcome
    };
    let (outcome, served) = tokio::join!(run, server);
    served.expect("the test agent serves");
    let recorded = seen
        .lock()
        .expect("the fixture lock is not poisoned")
        .clone();
    (outcome, recorded)
}

// A bound-then-released port: connecting to it fails, as an unreachable advertised URL does
pub(crate) fn unused_base() -> String {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("the fixture binds a port");
    format!(
        "http://{}",
        listener
            .local_addr()
            .expect("the fixture port has an address")
    )
}

pub(crate) fn card_json(endpoint: &str) -> String {
    serde_json::json!({
        "name": "fake-agent",
        "url": endpoint,
        "version": "1.0",
        "capabilities": {"streaming": true},
        "defaultInputModes": ["text"],
        "defaultOutputModes": ["text"],
        "skills": []
    })
    .to_string()
}

pub(crate) fn buffered_message(text: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": "rpc-1",
        "result": {"message": {"messageId": "m-1", "role": "ROLE_AGENT", "parts": [{"text": text}]}}
    })
    .to_string()
}

pub(crate) fn message_frame(text: &str) -> String {
    format!("data: {}\n\n", buffered_message(text))
}

pub(crate) fn status_frame(state: &str) -> String {
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "rpc-1",
        "result": {"statusUpdate": {
            "taskId": "t-1",
            "contextId": "c-1",
            "status": {"state": format!("TASK_STATE_{}", state.to_uppercase())}
        }}
    });
    format!("data: {payload}\n\n")
}

pub(crate) fn request(text: &str) -> SendMessageRequest {
    request_with_id("request-1", text)
}

pub(crate) fn request_with_id(id: &str, text: &str) -> SendMessageRequest {
    let body = serde_json::json!({
        "id": id,
        "params": {"message": {
            "kind": "message",
            "messageId": "m-1",
            "role": "user",
            "parts": [{"kind": "text", "text": text}]
        }}
    });
    serde_json::from_value(body).expect("the fixture request deserializes")
}

pub(crate) fn messages(text: &str) -> Vec<AgentJson> {
    serde_json::from_value(serde_json::json!([{"role": "user", "content": text}]))
        .expect("the fixture messages are JSON")
}

pub(crate) fn params_with_provider(provider: Option<AgentJson>) -> BTreeMap<String, AgentJson> {
    let mut params = litellm_params(&[("model", AgentJson::String("agent".to_owned()))]);
    if let Some(provider) = provider {
        params.insert("custom_llm_provider".to_owned(), provider);
    }
    params
}

pub(crate) fn pricing_params() -> BTreeMap<String, AgentJson> {
    let price = |text: &str| serde_json::from_str::<AgentJson>(text).expect("the price is JSON");
    litellm_params(&[
        ("cost_per_query", price("3")),
        ("input_cost_per_token", price("0.0001")),
        ("model", AgentJson::String("agent".to_owned())),
    ])
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture text is JSON")
}

pub(crate) fn pool() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .expect("the proxy policy resolves");
    ProxiedClient::outbound(&Arc::new(policy)).expect("the outbound client builds")
}

pub(crate) fn tls() -> TlsPolicy {
    TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "litellm/1.0".to_owned(),
    }
}

pub(crate) fn fixed_time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("the fixture time is RFC 3339")
}

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

pub(crate) fn logging_state(trace_id: Option<&str>) -> LoggingState {
    let mut correlation = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "gateway-model".to_owned(),
            call_type: "acompletion".to_owned(),
            start_time: fixed_time(),
            litellm_call_id: "call-1".to_owned(),
            function_id: "function-1".to_owned(),
            messages: None,
            stream: Some(false),
            litellm_trace_id: trace_id.map(str::to_owned),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut correlation,
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || "unused-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("the fixture logging state builds")
}

#[derive(Clone)]
pub(crate) struct LoggingSnapshot {
    pub(crate) model: String,
    pub(crate) call_type: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) litellm_trace_id: String,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) model_call_details: IndexMap<String, Json>,
}

#[derive(Clone)]
pub(crate) struct Recorded {
    pub(crate) call: StreamSuccessCall,
    pub(crate) logging: LoggingSnapshot,
}

pub(crate) struct RecordingDispatch {
    calls: Arc<Mutex<Vec<Recorded>>>,
}

pub(crate) fn recording_dispatch() -> (RecordingDispatch, Arc<Mutex<Vec<Recorded>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    (
        RecordingDispatch {
            calls: Arc::clone(&calls),
        },
        calls,
    )
}

#[async_trait]
impl StreamSuccessDispatch for RecordingDispatch {
    async fn dispatch(&mut self, logging: &mut LoggingState, call: StreamSuccessCall) {
        let snapshot = LoggingSnapshot {
            model: logging.model.clone(),
            call_type: logging.call_type.clone(),
            custom_llm_provider: logging.custom_llm_provider.clone(),
            litellm_trace_id: logging.litellm_trace_id.clone(),
            litellm_params: logging.litellm_params.clone(),
            optional_params: logging.optional_params.clone(),
            model_call_details: logging.model_call_details.clone(),
        };
        self.calls
            .lock()
            .expect("the fixture lock is not poisoned")
            .push(Recorded {
                call,
                logging: snapshot,
            });
    }
}

pub(crate) fn recorded(calls: &Arc<Mutex<Vec<Recorded>>>) -> Vec<Recorded> {
    calls
        .lock()
        .expect("the fixture lock is not poisoned")
        .clone()
}

pub(crate) fn header(seen: &[Seen], path: &str, name: &str) -> Option<String> {
    let entry = seen.iter().find(|entry| entry.path == path)?;
    entry.headers.get(name).cloned()
}

pub(crate) fn posted(seen: &[Seen]) -> &Seen {
    seen.iter()
        .find(|entry| entry.path == "/")
        .expect("the agent received the message")
}

pub(crate) fn card_requests(seen: &[Seen]) -> Vec<&Seen> {
    seen.iter()
        .filter(|entry| entry.path == CARD_PATH || entry.path == CUSTOM_CARD_PATH)
        .collect()
}

pub(crate) fn details(logging: &LoggingSnapshot, key: &str) -> Option<Json> {
    logging.model_call_details.get(key).cloned()
}

struct InlineCounter;

#[async_trait]
impl BlockingCounter for InlineCounter {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        job()
    }

    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}

pub(crate) fn counter() -> Counter {
    let settings = CounterSettings {
        disabled: false,
        max_exact_chars: NonZeroUsize::new(4_000_000).expect("the limit is positive"),
        chunk_chars: NonZeroUsize::new(1024).expect("the chunk is positive"),
    };
    let client = Arc::new(pool());
    let service = Arc::new(CountingService::new(
        4,
        Arc::new(InlineCounter),
        settings.encode_limits().expect("the limits fit"),
    ));
    let fetcher = Arc::new(TiktokenFetcher::new(
        Arc::clone(&client),
        None,
        Arc::clone(&service),
    ));
    let hub = hub_settings(Path::new("/nonexistent/betterlite-b4814"));
    let downloader = Arc::new(HubDownloader::new(
        Arc::new(hub.clone()),
        resolver(&hub, &client),
        Arc::new(Instant::now),
    ));
    let catalog = catalog();
    let selector = Arc::new(Selector::new(
        Arc::clone(&fetcher),
        downloader,
        Arc::clone(&catalog),
        Arc::clone(&service),
        false,
    ));
    Counter::new(selector, fetcher, service, catalog, settings)
}

fn catalog() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}

fn hub_settings(directory: &Path) -> HubSettings {
    HubSettings {
        endpoint: "http://127.0.0.1:1".to_owned(),
        offline: OfflineSetting::Online,
        etag_timeout: 10,
        download_timeout: 10,
        disable_implicit_token: false,
        env_token: None,
        hf_home: directory.to_path_buf(),
        hub_cache: directory.to_path_buf(),
        token_path: directory.join("token"),
        stored_tokens_path: directory.join("stored_tokens"),
        transfer: TransferSetting::Standard,
    }
}

fn resolver(settings: &HubSettings, client: &Arc<ProxiedClient>) -> Arc<HubTokenResolver> {
    let oidc = OidcSettings {
        resource: None,
        id_token: None,
        github_actions: false,
        request_url: None,
        request_token: None,
        device_code_client_id: String::new(),
    };
    let clock: Arc<dyn HubClock> = Arc::new(SystemHubClock::default());
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    Arc::new(HubTokenResolver::new(
        settings.clone(),
        oidc,
        Arc::clone(client),
        reader,
        clock,
    ))
}

// Owns every dependency the streaming entry borrows, so a test hands out one deps value per call
pub(crate) struct StreamFixture {
    pool: ProxiedClient,
    environment: BTreeMap<String, String>,
    tls: TlsPolicy,
    counter: Counter,
    generator: Box<dyn Fn() -> String + Sync>,
    issued: Arc<Mutex<Vec<String>>>,
    monotonic: Box<dyn FnMut() -> Duration + Send>,
    correlation: CorrelationContext,
    redactor: Redactor,
}

impl StreamFixture {
    pub(crate) fn new() -> Self {
        let issued = Arc::new(Mutex::new(Vec::new()));
        let generator = {
            let issued = Arc::clone(&issued);
            Box::new(move || {
                let mut ids = issued.lock().expect("the fixture lock is not poisoned");
                let id = format!("generated-{}", ids.len().saturating_add(1));
                ids.push(id.clone());
                id
            })
        };
        Self {
            pool: pool(),
            environment: BTreeMap::new(),
            tls: tls(),
            counter: counter(),
            generator,
            issued,
            monotonic: Box::new(|| Duration::ZERO),
            correlation: CorrelationContext::default(),
            redactor: redactor(),
        }
    }

    pub(crate) fn deps<Dispatch>(
        &mut self,
        dispatch: Dispatch,
    ) -> StreamingDeps<'_, fn() -> DateTime<FixedOffset>, Dispatch> {
        StreamingDeps {
            native: NativeDeps {
                http: &self.pool,
                environment: &self.environment,
                tls: &self.tls,
                counter: &self.counter,
                trace_ids: &*self.generator,
            },
            clock: fixed_time,
            monotonic: &mut *self.monotonic,
            correlation: &mut self.correlation,
            correlation_settings: CorrelationSettings::default(),
            redactor: &self.redactor,
            dispatch,
            max_retained_bytes: DEFAULT_MAX_RETAINED_BYTES,
        }
    }

    pub(crate) fn issued(&self) -> Vec<String> {
        self.issued
            .lock()
            .expect("the fixture lock is not poisoned")
            .clone()
    }
}
