#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::Lookup;
use super::adapter::AthinaAdapter;
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::http_analytics_loggers::athina::{AthinaLogger, AthinaSettings};
use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};
use crate::registry::catalog::{CatalogInputs, LoggerCatalog};

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "litellm_call_id": "call-1",
    "call_type": "completion",
    "stream": false,
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
    "metadata": {
        "user_api_key_user_id": "user-1",
        "requester_metadata": {"environment": "production", "prompt_slug": "greeting"}
    },
    "cache_hit": false,
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {"temperature": 0.2},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "Hello"}],
    "response": {
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}}],
        "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}
    }
}"#;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Recorded {
    url: String,
    headers: BTreeMap<String, String>,
    body: Json,
}

#[derive(Clone, Default)]
struct Captured {
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl Captured {
    fn requests(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the capture delivery failed")]
struct CaptureError;

impl Delivery for Captured {
    type Error = CaptureError;

    fn post(
        &self,
        request: PostRequest,
    ) -> impl Future<Output = Result<PostResponse, Self::Error>> + Send {
        let Ok(body) = serde_json::from_slice::<Json>(&request.body) else {
            return ready(Err(CaptureError));
        };
        let headers = request
            .headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (name.as_str().to_owned(), text.to_owned()))
            })
            .collect();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: request.url.to_string(),
                headers,
                body,
            });
        ready(Ok(PostResponse {
            status: 200,
            body: Vec::new(),
        }))
    }
}

fn inputs(environment: &[(&str, &str)]) -> CatalogInputs {
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
        env: Arc::new(
            environment
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<String, String>>(),
        ),
        callback_settings: BTreeMap::new(),
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

fn event_from(payload: &str, stream: bool) -> LoggerEvent {
    LoggerEvent {
        payload: Arc::new(serde_json::from_str(payload).expect("the fixture payload decodes")),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream,
        guardrail_snapshot: None,
    }
}

fn event(stream: bool) -> LoggerEvent {
    event_from(PAYLOAD, stream)
}

fn settings(key: Option<&str>) -> AthinaSettings {
    let environment = key.map_or_else(BTreeMap::new, |value| {
        BTreeMap::from([("ATHINA_API_KEY".to_owned(), value.to_owned())])
    });
    AthinaSettings::from_environment(&environment)
}

async fn captured_event(stream: bool, settings: AthinaSettings) -> Vec<Recorded> {
    let capture = Captured::default();
    let tracker = TaskTracker::new();
    let adapter = AthinaAdapter {
        logger: Arc::new(AthinaLogger {
            delivery: capture.clone(),
            settings,
        }),
        tracker: tracker.clone(),
    };
    adapter
        .log_success_event(&event(stream))
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    capture.requests()
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn field<'body>(body: &'body Json, name: &str) -> &'body Json {
    let Json::Object(fields) = body else {
        panic!("the posted body is a JSON object");
    };
    fields
        .get(name)
        .unwrap_or_else(|| panic!("the posted body carries {name}"))
}

#[test]
fn resolves_by_name_to_an_athina_logger() {
    let catalog = LoggerCatalog::new(inputs(&[("ATHINA_API_KEY", "athina-key")]));
    let logger = catalog
        .resolve_logger("athina")
        .expect("the name is available")
        .expect("the logger builds");
    assert_eq!(logger.callback_name(), "athina");
}

#[test]
fn the_logger_still_builds_without_an_api_key() {
    let catalog = LoggerCatalog::new(inputs(&[]));
    let logger = catalog
        .resolve_logger("athina")
        .expect("the name is available")
        .expect("LiteLLM builds AthinaLogger without a key");
    assert_eq!(logger.callback_name(), "athina");
}

fn assert_athina_fields(body: &Json) {
    assert_eq!(
        field(body, "language_model_id"),
        &Json::Text("gpt-4o".to_owned())
    );
    assert_eq!(field(body, "prompt_tokens"), &number(2));
    assert_eq!(field(body, "completion_tokens"), &number(3));
    assert_eq!(field(body, "total_tokens"), &number(5));
    assert_eq!(field(body, "response_time"), &number(1500));
    let Json::List(prompt) = field(body, "prompt") else {
        panic!("the posted prompt is the messages list");
    };
    assert_eq!(field(&prompt[0], "role"), &Json::Text("user".to_owned()));
    assert_eq!(
        field(&prompt[0], "content"),
        &Json::Text("Hello".to_owned())
    );
}

fn assert_request_fields(body: &Json) {
    let request = field(body, "request");
    assert_eq!(field(request, "model"), &Json::Text("gpt-4o".to_owned()));
    assert_eq!(field(request, "stream"), &Json::Bool(false));
    let Json::Object(request_fields) = request else {
        panic!("the request is a JSON object");
    };
    // LiteLLM only adds the buffered response to the kwargs of a streamed call
    assert!(!request_fields.contains_key("complete_streaming_response"));
    assert_request_metadata(body);
}

fn set_values(metadata: &Json) -> Json {
    let Json::Object(fields) = metadata else {
        panic!("the posted metadata is a JSON object");
    };
    Json::Object(
        fields
            .iter()
            .filter(|(_name, value)| !matches!(value, Json::Null))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}

fn assert_request_metadata(body: &Json) {
    let request = field(body, "request");
    assert_eq!(
        set_values(field(field(request, "litellm_params"), "metadata")),
        Json::Object(IndexMap::from([
            (
                "user_api_key_user_id".to_owned(),
                Json::Text("user-1".to_owned())
            ),
            (
                "requester_metadata".to_owned(),
                Json::Object(IndexMap::from([
                    (
                        "environment".to_owned(),
                        Json::Text("production".to_owned())
                    ),
                    ("prompt_slug".to_owned(), Json::Text("greeting".to_owned())),
                ]))
            ),
            (
                "environment".to_owned(),
                Json::Text("production".to_owned())
            ),
            ("prompt_slug".to_owned(), Json::Text("greeting".to_owned())),
        ]))
    );
    assert_eq!(
        field(field(body, "response"), "id"),
        &Json::Text("chatcmpl-1".to_owned())
    );
    assert_eq!(
        field(body, "environment"),
        &Json::Text("production".to_owned())
    );
    assert_eq!(
        field(body, "prompt_slug"),
        &Json::Text("greeting".to_owned())
    );
}

fn assert_mapped_body(body: &Json) {
    assert_athina_fields(body);
    assert_request_fields(body);
}

#[tokio::test]
async fn a_success_event_posts_the_mapped_fields() {
    let requests = captured_event(false, settings(Some("athina-key"))).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        "https://log.athina.ai/api/v1/log/inference"
    );
    assert_eq!(
        requests[0]
            .headers
            .get("athina-api-key")
            .map(String::as_str),
        Some("athina-key")
    );
    assert_eq!(
        requests[0].headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_mapped_body(&requests[0].body);
}

#[tokio::test]
async fn a_null_metadata_value_is_posted_as_null() {
    let capture = Captured::default();
    let tracker = TaskTracker::new();
    let adapter = AthinaAdapter {
        logger: Arc::new(AthinaLogger {
            delivery: capture.clone(),
            settings: settings(Some("athina-key")),
        }),
        tracker: tracker.clone(),
    };
    let with_null = event_from(
        &PAYLOAD.replace(r#""environment": "production""#, r#""environment": null"#),
        false,
    );
    adapter
        .log_success_event(&with_null)
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_eq!(requests.len(), 1);
    let body = &requests[0].body;
    assert_eq!(field(body, "environment"), &Json::Null);
    let metadata = field(field(field(body, "request"), "litellm_params"), "metadata");
    assert_eq!(field(metadata, "environment"), &Json::Null);
    assert_eq!(field(metadata, "user_api_key_org_id"), &Json::Null);
    assert_eq!(
        field(metadata, "prompt_slug"),
        &Json::Text("greeting".to_owned())
    );
}

#[tokio::test]
async fn an_async_success_event_posts_the_mapped_fields_once() {
    let capture = Captured::default();
    let tracker = TaskTracker::new();
    let adapter = AthinaAdapter {
        logger: Arc::new(AthinaLogger {
            delivery: capture.clone(),
            settings: settings(Some("athina-key")),
        }),
        tracker: tracker.clone(),
    };
    adapter
        .async_log_success_event(Arc::new(event(false)))
        .await
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_eq!(requests.len(), 1);
    assert_mapped_body(&requests[0].body);
}

#[test]
fn the_catalog_environment_reaches_the_settings() {
    let inputs = inputs(&[("ATHINA_API_KEY", "athina-key")]);
    let settings = AthinaSettings::from_environment(&Lookup(inputs.env.as_ref()));
    assert_eq!(
        settings.api_key.map(|key| key.expose_secret().to_owned()),
        Some("athina-key".to_owned())
    );
}

#[tokio::test]
async fn a_stream_chunk_event_sends_nothing() {
    let capture = Captured::default();
    let tracker = TaskTracker::new();
    let adapter = AthinaAdapter {
        logger: Arc::new(AthinaLogger {
            delivery: capture.clone(),
            settings: settings(Some("athina-key")),
        }),
        tracker: tracker.clone(),
    };
    adapter
        .log_stream_event(&event(true))
        .expect("the chunk is accepted");
    tracker.close();
    tracker.wait().await;
    assert!(capture.requests().is_empty());
}

#[test]
fn an_invalid_start_time_is_reported() {
    let capture = Captured::default();
    let adapter = AthinaAdapter {
        logger: Arc::new(AthinaLogger {
            delivery: capture,
            settings: settings(Some("athina-key")),
        }),
        tracker: TaskTracker::new(),
    };
    let broken = event_from(
        &PAYLOAD.replace("1704067200.0", "100000000000000000000"),
        false,
    );
    let error = adapter
        .log_success_event(&broken)
        .expect_err("the event is rejected");
    assert!(matches!(error, LoggerError::Delivery { callback, .. } if callback == "athina"));
}

#[tokio::test]
async fn a_stream_success_event_posts_the_complete_response() {
    let requests = captured_event(true, settings(Some("athina-key"))).await;
    assert_eq!(requests.len(), 1);
    let body = &requests[0].body;
    assert_eq!(field(field(body, "request"), "stream"), &Json::Bool(true));
    assert_eq!(
        field(body, "language_model_id"),
        &Json::Text("gpt-4o".to_owned())
    );
    assert_eq!(field(body, "total_tokens"), &number(5));
    assert_eq!(
        field(field(body, "response"), "id"),
        &Json::Text("chatcmpl-1".to_owned())
    );
}

#[tokio::test]
async fn a_missing_api_key_sends_nothing() {
    let requests = captured_event(false, settings(None)).await;
    assert!(requests.is_empty());
}
