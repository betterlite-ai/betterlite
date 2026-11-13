#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::adapter::{ArgillaAdapter, Pending};
use super::build;
use crate::argilla_logging::logger::RandomSource;
use crate::argilla_logging::settings::{ArgillaSettings, ConstructorArguments, PayloadField};
use crate::argilla_logging::transport::{
    ArgillaMethod, ArgillaRequest, ArgillaResponse, Transport,
};
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::registry::catalog::{CatalogError, CatalogInputs, LoggerCatalog};

const DATASET_BODY: &str = r#"{"items": [{"id": "ds-1"}]}"#;

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "litellm_call_id": "call-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.5,
    "model_map_information": {},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {"user_api_key_user_id": "user-1"},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "Hello"}],
    "response": {
        "id": "chatcmpl-1",
        "choices": [{"message": {"role": "assistant", "content": "Hi"}}],
        "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}
    }
}"#;

const CONFIGURED: &str =
    "callback_settings:\n  argilla:\n    input: messages\n    output: response\n";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Call {
    method: ArgillaMethod,
    url: String,
    api_key: String,
    body: Option<Json>,
}

#[derive(Clone, Default)]
struct Capture {
    calls: Arc<Mutex<Vec<Call>>>,
}

impl Capture {
    fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn sent(&self, method: ArgillaMethod) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|call| call.method == method)
            .collect()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the capture transport failed")]
struct CaptureError;

impl Transport for Capture {
    type Error = CaptureError;

    fn send(
        &self,
        request: ArgillaRequest,
    ) -> impl Future<Output = Result<ArgillaResponse, CaptureError>> + Send {
        let method = request.method;
        let decoded = request
            .body
            .as_deref()
            .map(serde_json::from_slice::<Json>)
            .transpose();
        let Ok(body) = decoded else {
            return ready(Err(CaptureError));
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Call {
                method,
                url: request.url,
                api_key: request.api_key.expose_secret().to_owned(),
                body,
            });
        let answer = match method {
            ArgillaMethod::Get => DATASET_BODY,
            ArgillaMethod::Post | ArgillaMethod::Put => "",
        };
        ready(Ok(ArgillaResponse {
            status: 200,
            body: answer.to_owned(),
        }))
    }
}

struct FixedSample(f64);

impl RandomSource for FixedSample {
    fn sample(&self) -> f64 {
        self.0
    }
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn transformation() -> Json {
    Json::Object(IndexMap::from([
        ("input".to_owned(), Json::Text("messages".to_owned())),
        ("output".to_owned(), Json::Text("response".to_owned())),
    ]))
}

fn full_environment() -> BTreeMap<String, String> {
    // one record per flush so a single event posts the batch
    environment(&[
        ("ARGILLA_API_KEY", "argilla-key"),
        ("ARGILLA_BATCH_SIZE", "1"),
    ])
}

fn settings(overrides: &[(&str, &str)]) -> ArgillaSettings {
    let mut values = full_environment();
    values.extend(environment(overrides));
    ArgillaSettings::from_environment(
        &values,
        &ConstructorArguments::default(),
        Some(&transformation()),
    )
    .expect("the fixture settings are valid")
}

fn inputs(source: &str, pairs: &[(&str, &str)]) -> CatalogInputs {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("the fixture config loads");
    let callback_settings = config.sections.callback_settings.callback_settings.clone();
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
    CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("http client"),
        env: Arc::new(environment(pairs)),
        callback_settings,
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

fn configured_inputs() -> CatalogInputs {
    inputs(
        CONFIGURED,
        &[
            ("ARGILLA_API_KEY", "argilla-key"),
            ("ARGILLA_BATCH_SIZE", "1"),
        ],
    )
}

fn event_from(payload: &str) -> LoggerEvent {
    LoggerEvent {
        payload: Arc::new(serde_json::from_str(payload).expect("the fixture payload decodes")),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    }
}

fn event() -> LoggerEvent {
    event_from(PAYLOAD)
}

fn adapter(capture: &Capture, settings: ArgillaSettings) -> ArgillaAdapter<Capture, FixedSample> {
    ArgillaAdapter::new(
        Pending {
            settings,
            transport: capture.clone(),
            random: FixedSample(0.0),
        },
        TaskTracker::new(),
    )
}

async fn drain(adapter: &ArgillaAdapter<Capture, FixedSample>) {
    adapter.tracker.close();
    adapter.tracker.wait().await;
}

fn fields_of(item: &Json) -> &Json {
    match item {
        Json::Object(object) => object
            .get("fields")
            .expect("the record carries its mapped fields"),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the record is an object")
        }
    }
}

fn field<'json>(object: &'json Json, name: &str) -> &'json Json {
    match object {
        Json::Object(fields) => fields
            .get(name)
            .unwrap_or_else(|| panic!("the object carries {name}")),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the value is an object")
        }
    }
}

#[tokio::test]
async fn resolves_by_name_to_an_argilla_logger() {
    let inputs = configured_inputs();
    let tracker = inputs.tracker.clone();
    let catalog = LoggerCatalog::new(inputs);
    let logger = catalog
        .resolve_logger("argilla")
        .expect("the name is available")
        .expect("the logger builds");
    assert_eq!(logger.callback_name(), "argilla");
    assert_eq!(tracker.len(), 1);
}

#[test]
fn a_missing_api_key_gives_init() {
    assert!(matches!(
        build(&inputs(CONFIGURED, &[("ARGILLA_BATCH_SIZE", "1")])),
        Err(CatalogError::Init { message })
            if message == "Invalid Argilla API Key given. _credentials_api_key=None."
    ));
}

#[test]
fn a_missing_transformation_gives_init() {
    assert!(matches!(
        build(&inputs("", &[("ARGILLA_API_KEY", "argilla-key")])),
        Err(CatalogError::Init { message })
            if message
                == "'litellm.argilla_transformation_object' is required, to log your payload to Argilla."
    ));
}

#[test]
fn a_transformation_with_an_unknown_field_gives_init() {
    let config = "callback_settings:\n  argilla:\n    input: unknown\n";
    assert!(matches!(
        build(&inputs(config, &[("ARGILLA_API_KEY", "argilla-key")])),
        Err(CatalogError::Init { message }) if message.contains("SUPPORTED_PAYLOAD_FIELDS")
    ));
}

#[test]
fn the_transformation_keys_become_the_record_fields() {
    let settings = settings(&[]);
    assert_eq!(
        settings
            .transformation
            .iter()
            .map(|(name, field)| (name.as_str(), *field))
            .collect::<Vec<_>>(),
        vec![
            ("input", PayloadField::Messages),
            ("output", PayloadField::Response)
        ]
    );
}

#[tokio::test]
async fn a_sync_success_event_posts_the_mapped_record() {
    let capture = Capture::default();
    let adapter = adapter(&capture, settings(&[]));
    adapter
        .log_success_event(&event())
        .expect("the event is accepted");
    drain(&adapter).await;

    let lookups = capture.sent(ArgillaMethod::Get);
    assert_eq!(lookups.len(), 1);
    assert_eq!(lookups[0].api_key, "argilla-key");
    let posts = capture.sent(ArgillaMethod::Post);
    assert_eq!(posts.len(), 1);
    assert_eq!(
        posts[0].url,
        "http://localhost:6900//api/v1/datasets/ds-1/records/bulk"
    );
    let Json::List(records) = posts[0]
        .body
        .as_ref()
        .expect("the sync body is a JSON array")
    else {
        panic!("the sync body is an array");
    };
    assert_eq!(records.len(), 1);
    let fields = fields_of(&records[0]);
    assert_eq!(
        field(fields, "input"),
        &Json::List(vec![Json::Object(IndexMap::from([
            ("role".to_owned(), Json::Text("user".to_owned())),
            ("content".to_owned(), Json::Text("Hello".to_owned())),
        ]))])
    );
    assert_eq!(field(fields, "output"), &Json::Text("Hi".to_owned()));
}

#[tokio::test]
async fn an_async_success_event_posts_the_mapped_record() {
    let capture = Capture::default();
    let adapter = adapter(&capture, settings(&[]));
    adapter
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is logged");

    assert_eq!(capture.sent(ArgillaMethod::Get).len(), 1);
    let posts = capture.sent(ArgillaMethod::Put);
    assert_eq!(posts.len(), 1);
    assert_eq!(
        posts[0].url,
        "http://localhost:6900//api/v1/datasets/ds-1/records/bulk"
    );
    let body = posts[0]
        .body
        .as_ref()
        .expect("the async body is a JSON object");
    let Json::List(records) = field(body, "items") else {
        panic!("the async body carries an items array");
    };
    assert_eq!(
        field(fields_of(&records[0]), "output"),
        &Json::Text("Hi".to_owned())
    );
}

#[tokio::test]
async fn an_async_failure_event_posts_the_mapped_record() {
    let capture = Capture::default();
    let adapter = adapter(&capture, settings(&[]));
    adapter
        .async_log_failure_event(Arc::new(event()))
        .await
        .expect("the event is logged");

    assert_eq!(capture.sent(ArgillaMethod::Get).len(), 1);
    let posts = capture.sent(ArgillaMethod::Put);
    assert_eq!(posts.len(), 1);
    let body = posts[0]
        .body
        .as_ref()
        .expect("the async body is a JSON object");
    let Json::List(records) = field(body, "items") else {
        panic!("the async body carries an items array");
    };
    assert!(!matches!(
        field(fields_of(&records[0]), "input"),
        Json::Null
    ));
}

#[tokio::test]
async fn a_sampled_out_sync_event_sends_nothing() {
    let capture = Capture::default();
    let adapter = ArgillaAdapter::new(
        Pending {
            settings: settings(&[]),
            transport: capture.clone(),
            random: FixedSample(2.0),
        },
        TaskTracker::new(),
    );
    adapter
        .log_success_event(&event())
        .expect("the event is accepted");
    drain(&adapter).await;
    // the dataset lookup runs before the sampler, as ArgillaLogger.__init__ resolves it whatever the sampler decides
    assert_eq!(capture.sent(ArgillaMethod::Get).len(), 1);
    assert!(capture.sent(ArgillaMethod::Post).is_empty());
    assert!(capture.sent(ArgillaMethod::Put).is_empty());
}

#[tokio::test(start_paused = true)]
async fn the_flush_loop_stops_on_shutdown() {
    let inputs = configured_inputs();
    let _logger = build(&inputs).expect("the logger builds");
    inputs.shutdown.cancel();
    inputs.tracker.close();
    let awaited = timeout(Duration::from_secs(1), inputs.tracker.wait()).await;
    awaited.expect("the flush loop stops on shutdown");
}

#[tokio::test]
async fn the_flush_loop_resolves_the_dataset_on_its_first_tick() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let address = listener.local_addr().expect("the bound address");
    let base = format!("http://{address}");
    let inputs = inputs(
        CONFIGURED,
        &[
            ("ARGILLA_API_KEY", "argilla-key"),
            ("ARGILLA_BASE_URL", base.as_str()),
            // one tick is a real second, so the test waits on the loop instead of on a paused clock
            ("DEFAULT_FLUSH_INTERVAL_SECONDS", "1"),
        ],
    );
    let _logger = build(&inputs).expect("the logger builds");

    let serve = async {
        let accepted = timeout(Duration::from_secs(10), listener.accept()).await;
        let (mut stream, _) = accepted
            .expect("the flush loop connects")
            .expect("one request");
        let head = read_head(&mut stream).await;
        answer(&mut stream, DATASET_BODY).await;
        head
    };
    let head = serve.await;
    assert!(head.contains("GET /api/v1/me/datasets?name=litellm-completion"));
}

async fn read_head(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    loop {
        let mut chunk = [0_u8; 1024];
        let read = stream.read(&mut chunk).await.expect("a request read");
        assert!(read > 0, "the client closed before sending a request");
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let end = position.saturating_add(4);
            return String::from_utf8(buffer[..end].to_vec()).expect("a UTF-8 request head");
        }
    }
}

async fn answer(stream: &mut TcpStream, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("the response write");
    stream.flush().await.expect("the response flush");
}
