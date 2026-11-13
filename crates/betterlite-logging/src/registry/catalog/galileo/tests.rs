#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{Result as IoResult, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::MakeWriter;

use super::adapter::GalileoAdapter;
use super::payload::response_object;
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::galileo::delivery::config::GalileoConfig;
use crate::galileo::delivery::headers::Header;
use crate::galileo::delivery::transport::{HttpResponse, TraceIds, Transport, TransportError};
use crate::galileo::delivery::{GalileoDelivery, State};
use crate::galileo_payloads::content::ResponseObj;
use crate::registry::catalog::{CatalogInputs, LoggerCatalog};

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
    "model_parameters": {"temperature": 0.2},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "Hello"}],
    "response": {
        "id": "chatcmpl-1",
        "choices": [{"message": {"role": "assistant", "content": "Hi"}}],
        "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}
    }
}"#;

const CONFIGURED: &[(&str, &str)] = &[
    ("GALILEO_API_KEY", "key-0123456789"),
    ("GALILEO_PROJECT_ID", "project-1"),
];

// message_model_dump's record for {"role": "assistant", "content": "Hi"}, the tool_calls shape litellm.types.utils.Message declares.
const CHAT_OUTPUT: &str = r#"{"content": "Hi", "role": "assistant", "tool_calls": null, "function_call": null, "provider_specific_fields": null}"#;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Posted {
    url: String,
    headers: Vec<(String, String)>,
    payload: Option<IndexMap<String, Json>>,
}

#[derive(Clone, Default)]
struct Capture {
    calls: Arc<Mutex<Vec<Posted>>>,
}

impl Capture {
    fn calls(&self) -> Vec<Posted> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn answer(
        &self,
        url: &str,
        headers: &[Header],
        payload: Option<&IndexMap<String, Json>>,
    ) -> HttpResponse {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Posted {
                url: url.to_owned(),
                headers: headers
                    .iter()
                    .map(|header| (header.name.clone(), header.value.expose_secret().to_owned()))
                    .collect(),
                payload: payload.cloned(),
            });
        HttpResponse {
            status: 200,
            body: "{}".to_owned(),
        }
    }
}

impl Transport for Capture {
    fn get(&self, url: &str, headers: &[Header]) -> Result<HttpResponse, TransportError> {
        Ok(self.answer(url, headers, None))
    }

    fn post_form(
        &self,
        url: &str,
        headers: &[Header],
        _form: &[(&str, &str)],
    ) -> Result<HttpResponse, TransportError> {
        Ok(self.answer(url, headers, None))
    }

    fn post_json(
        &self,
        url: &str,
        headers: &[Header],
        payload: &IndexMap<String, Json>,
    ) -> Result<HttpResponse, TransportError> {
        Ok(self.answer(url, headers, Some(payload)))
    }
}

#[derive(Default)]
struct CountingIds(AtomicUsize);

impl TraceIds for CountingIds {
    fn next_id(&self) -> String {
        format!("id-{}", self.0.fetch_add(1, Ordering::SeqCst))
    }
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn delivery(pairs: &[(&str, &str)]) -> GalileoDelivery {
    GalileoDelivery {
        config: GalileoConfig::from_environment(&environment(pairs)),
        state: Mutex::new(State::default()),
    }
}

fn adapter(
    delivery: GalileoDelivery,
    capture: &Capture,
    shutdown: CancellationToken,
) -> GalileoAdapter {
    GalileoAdapter {
        delivery: Arc::new(delivery),
        transport: Arc::new(capture.clone()),
        ids: Arc::new(CountingIds::default()),
        tracker: TaskTracker::new(),
        shutdown,
    }
}

fn inputs(pairs: &[(&str, &str)]) -> CatalogInputs {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("valid proxy policy"),
    );
    CatalogInputs {
        config: Arc::new(
            ConfigLoader {
                lookup: BTreeMap::new(),
            }
            .load("")
            .expect("valid config"),
        ),
        http: ProxiedClient::outbound(&policy).expect("http client"),
        env: Arc::new(environment(pairs)),
        callback_settings: BTreeMap::new(),
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
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

fn field<'json>(value: &'json Json, name: &str) -> &'json Json {
    match value {
        Json::Object(fields) => fields
            .get(name)
            .unwrap_or_else(|| panic!("the payload carries {name}")),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the value is an object")
        }
    }
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn trace_of(payload: &IndexMap<String, Json>) -> Json {
    let Some(Json::List(traces)) = payload.get("traces") else {
        panic!("the payload carries a traces list");
    };
    traces.first().cloned().expect("one trace")
}

#[tokio::test]
async fn resolves_by_name_to_a_galileo_logger() {
    let catalog = LoggerCatalog::new(inputs(CONFIGURED));
    let logger = catalog
        .resolve_logger("galileo")
        .expect("the name is available")
        .expect("the logger builds");
    assert_eq!(logger.callback_name(), "galileo");
}

#[tokio::test]
async fn a_success_event_posts_the_mapped_record() {
    let capture = Capture::default();
    let adapter = adapter(delivery(CONFIGURED), &capture, CancellationToken::new());
    adapter
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is logged");

    let calls = capture.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].url,
        "https://api.galileo.ai/ingest/traces/project-1"
    );
    assert!(
        calls[0]
            .headers
            .contains(&("Galileo-API-Key".to_owned(), "key-0123456789".to_owned()))
    );
    let payload = calls[0].payload.as_ref().expect("the post carries a body");
    assert_success_trace(&trace_of(payload));
}

fn assert_success_trace(trace: &Json) {
    assert_eq!(field(trace, "type"), &Json::Text("trace".to_owned()));
    assert_eq!(field(trace, "id"), &Json::Text("id-0".to_owned()));
    assert_eq!(field(trace, "name"), &Json::Text("completion".to_owned()));
    assert_eq!(field(trace, "input"), &Json::Text("Hello".to_owned()));
    assert_eq!(
        field(trace, "created_at"),
        &Json::Text("2024-01-01T00:00:00Z".to_owned())
    );
    let Json::List(spans) = field(trace, "spans") else {
        panic!("the trace nests one span");
    };
    let span = spans.first().expect("one span");
    assert_eq!(field(span, "trace_id"), &Json::Text("id-0".to_owned()));
    assert_eq!(field(span, "id"), &Json::Text("id-1".to_owned()));
    assert_eq!(field(span, "model"), &Json::Text("gpt-4o".to_owned()));
    let metrics = field(span, "metrics");
    assert_eq!(field(metrics, "num_input_tokens"), &number(2));
    assert_eq!(field(metrics, "num_output_tokens"), &number(3));
    assert_eq!(field(metrics, "num_total_tokens"), &number(5));
    assert_eq!(field(metrics, "duration_ns"), &number(1_500_000_000));
    assert_eq!(
        field(span, "output"),
        &Json::Object(IndexMap::from([
            ("role".to_owned(), Json::Text("assistant".to_owned())),
            ("content".to_owned(), Json::Text(CHAT_OUTPUT.to_owned())),
        ]))
    );
}

#[tokio::test]
async fn an_embedding_event_marks_the_output() {
    let capture = Capture::default();
    let adapter = adapter(delivery(CONFIGURED), &capture, CancellationToken::new());
    let fixture = PAYLOAD.replace(
        r#""call_type": "completion""#,
        r#""call_type": "embedding""#,
    );
    adapter
        .async_log_success_event(Arc::new(event_from(&fixture)))
        .await
        .expect("the event is logged");

    let calls = capture.calls();
    assert_eq!(calls.len(), 1);
    let body = calls[0].payload.as_ref().expect("the post carries a body");
    assert_eq!(
        field(&trace_of(body), "output"),
        &Json::Text("embedding-output".to_owned())
    );
}

#[tokio::test]
async fn a_failure_event_posts_nothing() {
    let capture = Capture::default();
    let adapter = adapter(delivery(CONFIGURED), &capture, CancellationToken::new());
    adapter
        .async_log_failure_event(Arc::new(event()))
        .await
        .expect("the failure event is diagnostic only");
    assert!(capture.calls().is_empty());
}

#[tokio::test]
async fn an_unconfigured_logger_posts_nothing() {
    let capture = Capture::default();
    let adapter = adapter(
        delivery(&[("GALILEO_API_KEY", "key-0123456789")]),
        &capture,
        CancellationToken::new(),
    );
    adapter
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    assert!(capture.calls().is_empty());
}

#[tokio::test]
async fn a_cancelled_adapter_posts_nothing() {
    let capture = Capture::default();
    let shutdown = CancellationToken::new();
    shutdown.cancel();
    let adapter = adapter(delivery(CONFIGURED), &capture, shutdown);
    adapter
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    assert!(capture.calls().is_empty());
}

#[tokio::test]
async fn a_failure_event_writes_the_diagnostic_line() {
    let capture = Capture::default();
    let adapter = adapter(delivery(CONFIGURED), &capture, CancellationToken::new());
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    {
        let _guard = set_default(subscriber);
        adapter
            .async_log_failure_event(Arc::new(event()))
            .await
            .expect("the failure event is diagnostic only");
    };
    assert!(buffer.text().contains("On Async Failure"));
    assert!(capture.calls().is_empty());
}

#[test]
fn the_production_trace_ids_are_distinct_uuids() {
    let ids = super::UuidTraceIds;
    let first = ids.next_id();
    let second = ids.next_id();
    uuid::Uuid::parse_str(&first).expect("the first id is a UUID");
    uuid::Uuid::parse_str(&second).expect("the second id is a UUID");
    assert_ne!(first, second);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Chat,
    TextCompletion,
    Embedding,
    Speech,
    Image,
    Transcription,
    Rerank,
    Responses,
    Realtime,
    PassThrough,
    Generic,
    Unknown,
}

fn family_of(object: &ResponseObj) -> Family {
    match object {
        ResponseObj::Chat { .. } => Family::Chat,
        ResponseObj::TextCompletion { .. } => Family::TextCompletion,
        ResponseObj::Embedding => Family::Embedding,
        ResponseObj::Speech => Family::Speech,
        ResponseObj::Image { .. } => Family::Image,
        ResponseObj::Transcription { .. } => Family::Transcription,
        ResponseObj::Rerank { .. } => Family::Rerank,
        ResponseObj::ResponsesApi { .. } => Family::Responses,
        ResponseObj::RealtimeList { .. } => Family::Realtime,
        ResponseObj::PassThroughDict { .. } => Family::PassThrough,
        ResponseObj::GenericDict { .. } => Family::Generic,
        ResponseObj::Unknown => Family::Unknown,
    }
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the test JSON decodes")
}

#[rstest]
#[case::embedding("embedding", "{}", Family::Embedding)]
#[case::async_embedding("aembedding", "{}", Family::Embedding)]
#[case::speech("speech", "{}", Family::Speech)]
#[case::async_speech("aspeech", "{}", Family::Speech)]
#[case::image("image_generation", r#"{"data": [{"url": "u"}]}"#, Family::Image)]
#[case::async_image("aimage_generation", r#"{"data": []}"#, Family::Image)]
#[case::image_edit("image_edit", r#"{"data": [{"url": "u"}]}"#, Family::Image)]
#[case::async_image_edit("aimage_edit", r#"{"data": []}"#, Family::Image)]
#[case::transcription("transcription", r#"{"text": "hello"}"#, Family::Transcription)]
#[case::async_transcription("atranscription", r#"{"text": "hello"}"#, Family::Transcription)]
#[case::rerank("rerank", r#"{"results": [{"index": 0}]}"#, Family::Rerank)]
#[case::async_rerank("arerank", r#"{"results": []}"#, Family::Rerank)]
#[case::responses("responses", r#"{"output": [{"type": "message"}]}"#, Family::Responses)]
#[case::async_responses("aresponses", r#"{"output": []}"#, Family::Responses)]
#[case::realtime("_arealtime", "[1, 2]", Family::Realtime)]
#[case::pass_through("pass_through_endpoint", r#"{"response": "ok"}"#, Family::PassThrough)]
#[case::chat(
    "completion",
    r#"{"choices": [{"message": {"role": "assistant", "content": "Hi"}}]}"#,
    Family::Chat
)]
#[case::text_completion(
    "completion",
    r#"{"choices": [{"text": "Hi"}]}"#,
    Family::TextCompletion
)]
#[case::generic_dict("completion", r#"{"other": 1}"#, Family::Generic)]
#[case::unknown("completion", "null", Family::Unknown)]
fn every_call_type_selects_its_response_family(
    #[case] call_type: &str,
    #[case] response: &str,
    #[case] expected: Family,
) {
    assert_eq!(
        family_of(&response_object(call_type, &json(response))),
        expected
    );
}

#[test]
fn the_chat_message_comes_from_the_first_choice() {
    let object = response_object(
        "completion",
        &json(r#"{"choices": [{"message": {"role": "assistant", "content": "Hi"}}]}"#),
    );
    let ResponseObj::Chat { message } = object else {
        panic!("the chat family is selected");
    };
    assert_eq!(
        message,
        Some(json(r#"{"role": "assistant", "content": "Hi"}"#))
    );
}

#[test]
fn a_text_completion_carries_its_text() {
    let object = response_object("completion", &json(r#"{"choices": [{"text": "Hi"}]}"#));
    let ResponseObj::TextCompletion { text } = object else {
        panic!("the text completion family is selected");
    };
    assert_eq!(text, Some("Hi".to_owned()));
}

#[test]
fn an_image_carries_its_data() {
    let object = response_object("image_generation", &json(r#"{"data": [{"url": "u"}]}"#));
    let ResponseObj::Image { data } = object else {
        panic!("the image family is selected");
    };
    assert_eq!(data, Some(json(r#"[{"url": "u"}]"#)));
}

#[test]
fn a_transcription_carries_its_text() {
    let object = response_object("transcription", &json(r#"{"text": "hello"}"#));
    let ResponseObj::Transcription { text } = object else {
        panic!("the transcription family is selected");
    };
    assert_eq!(text, Some("hello".to_owned()));
}

#[test]
fn a_rerank_carries_its_results() {
    let object = response_object("rerank", &json(r#"{"results": [{"index": 0}]}"#));
    let ResponseObj::Rerank { results } = object else {
        panic!("the rerank family is selected");
    };
    assert_eq!(results, json(r#"[{"index": 0}]"#));
}

#[test]
fn a_responses_call_carries_its_output() {
    let object = response_object("responses", &json(r#"{"output": [{"type": "message"}]}"#));
    let ResponseObj::ResponsesApi { output } = object else {
        panic!("the responses family is selected");
    };
    assert_eq!(output, json(r#"[{"type": "message"}]"#));
}

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("the captured output is UTF-8")
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
