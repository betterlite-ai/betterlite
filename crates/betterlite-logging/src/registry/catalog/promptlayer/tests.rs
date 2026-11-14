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
use super::adapter::PromptLayerAdapter;
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};
use crate::http_analytics_loggers::promptlayer::{PromptLayerLogger, PromptLayerSettings};
use crate::registry::catalog::{CatalogInputs, LoggerCatalog};

const RESPONSE: &str = r#"{"id": "chatcmpl-1", "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}}"#;

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
    "endTime": 1704067201.5,
    "completionStartTime": 1704067200.5,
    "response_time": 1.5,
    "model_map_information": {},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {
        "user_api_key_user_id": "user-1",
        "requester_metadata": {"pl_tags": ["tag-1", "tag-2"]}
    },
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {"temperature": 0.2},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "Hello"}],
    "response": {"id": "chatcmpl-1", "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}}
}"#;

const SUCCESS: &str = r#"{"success": true}"#;
const SUCCESS_WITH_REQUEST: &str = r#"{"success": true, "request_id": "req-1"}"#;
const UNSUCCESSFUL: &str = r#"{"success": false}"#;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Recorded {
    url: String,
    headers: BTreeMap<String, String>,
    body: Json,
}

#[derive(Clone)]
struct Capture {
    requests: Arc<Mutex<Vec<Recorded>>>,
    answer: Arc<Mutex<Json>>,
}

impl Capture {
    fn answering(answer: &str) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            answer: Arc::new(Mutex::new(
                serde_json::from_str(answer).expect("the answer fixture decodes"),
            )),
        }
    }

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

impl Delivery for Capture {
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
        let answer = self
            .answer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match serde_json::to_vec(&answer) {
            Ok(encoded) => ready(Ok(PostResponse {
                status: 200,
                body: encoded,
            })),
            Err(_source) => ready(Err(CaptureError)),
        }
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

fn settings(api_key: Option<&str>) -> PromptLayerSettings {
    let environment = api_key.map_or_else(BTreeMap::new, |value| {
        BTreeMap::from([("PROMPTLAYER_API_KEY".to_owned(), value.to_owned())])
    });
    PromptLayerSettings::from_environment(&environment)
}

fn adapter(capture: Capture, settings: PromptLayerSettings) -> PromptLayerAdapter<Capture> {
    PromptLayerAdapter {
        logger: Arc::new(PromptLayerLogger {
            delivery: capture,
            settings,
        }),
        tracker: TaskTracker::new(),
    }
}

async fn deliver(event: &LoggerEvent, adapter: &PromptLayerAdapter<Capture>) {
    adapter
        .log_success_event(event)
        .expect("the event is accepted");
    adapter.tracker.close();
    adapter.tracker.wait().await;
}

fn field<'body>(body: &'body Json, name: &str) -> &'body Json {
    let Json::Object(fields) = body else {
        panic!("the posted body is a JSON object");
    };
    fields
        .get(name)
        .unwrap_or_else(|| panic!("the posted body carries {name}"))
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

#[test]
fn resolves_by_name_to_a_promptlayer_logger() {
    let catalog = LoggerCatalog::new(inputs(&[("PROMPTLAYER_API_KEY", "pl-key")]));
    let logger = catalog
        .resolve_logger("promptlayer")
        .expect("the name is available")
        .expect("the logger builds");
    assert_eq!(logger.callback_name(), "promptlayer");
}

#[test]
fn the_logger_still_builds_without_an_api_key() {
    let catalog = LoggerCatalog::new(inputs(&[]));
    let logger = catalog
        .resolve_logger("promptlayer")
        .expect("the name is available")
        .expect("LiteLLM builds PromptLayerLogger without a key");
    assert_eq!(logger.callback_name(), "promptlayer");
}

#[test]
fn the_catalog_environment_reaches_the_settings() {
    let inputs = inputs(&[("PROMPTLAYER_API_KEY", "pl-key")]);
    let settings = PromptLayerSettings::from_environment(&Lookup(inputs.env.as_ref()));
    assert_eq!(
        settings.api_key.map(|key| key.expose_secret().to_owned()),
        Some("pl-key".to_owned())
    );
}

fn assert_track_request(request: &Recorded) {
    assert_eq!(
        request.url,
        "https://api.promptlayer.com/rest/track-request"
    );
    assert_eq!(
        request.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    let body = &request.body;
    assert_eq!(
        field(body, "function_name"),
        &Json::Text("openai.ChatCompletion.create".to_owned())
    );
    assert_eq!(field(body, "api_key"), &Json::Text("pl-key".to_owned()));
    assert_eq!(field(body, "request_start_time"), &number(1_704_067_200));
    assert_eq!(field(body, "request_end_time"), &number(1_704_067_201));
    assert_eq!(
        field(body, "tags"),
        &Json::List(vec![
            Json::Text("tag-1".to_owned()),
            Json::Text("tag-2".to_owned())
        ])
    );
    let kwargs = field(body, "kwargs");
    assert_eq!(field(kwargs, "model"), &Json::Text("gpt-4o".to_owned()));
    assert_eq!(
        field(kwargs, "messages"),
        &Json::List(vec![Json::Object(IndexMap::from([
            ("role".to_owned(), Json::Text("user".to_owned())),
            ("content".to_owned(), Json::Text("Hello".to_owned())),
        ]))])
    );
    assert_eq!(
        serde_json::to_string(field(kwargs, "temperature")).expect("a printable number"),
        "0.2"
    );
    assert_eq!(
        field(body, "request_response"),
        &serde_json::from_str::<Json>(RESPONSE).expect("the response fixture decodes")
    );
}

#[tokio::test]
async fn a_success_event_posts_the_mapped_fields() {
    let capture = Capture::answering(SUCCESS);
    let adapter = adapter(capture.clone(), settings(Some("pl-key")));
    deliver(&event(false), &adapter).await;
    let requests = capture.requests();
    assert_eq!(requests.len(), 1);
    assert_track_request(&requests[0]);
}

#[tokio::test]
async fn an_async_success_event_posts_the_mapped_fields_once() {
    let capture = Capture::answering(SUCCESS);
    let adapter = adapter(capture.clone(), settings(Some("pl-key")));
    adapter
        .async_log_success_event(Arc::new(event(false)))
        .await
        .expect("the event is accepted");
    adapter.tracker.close();
    adapter.tracker.wait().await;
    let requests = capture.requests();
    assert_eq!(requests.len(), 1);
    assert_track_request(&requests[0]);
}

#[tokio::test]
async fn a_success_event_posts_the_metadata_when_a_request_id_comes_back() {
    let capture = Capture::answering(SUCCESS_WITH_REQUEST);
    let adapter = adapter(capture.clone(), settings(Some("pl-key")));
    deliver(&event(false), &adapter).await;
    let requests = capture.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].url,
        "https://api.promptlayer.com/rest/track-metadata"
    );
    let body = &requests[1].body;
    assert_eq!(field(body, "request_id"), &Json::Text("req-1".to_owned()));
    assert_eq!(field(body, "api_key"), &Json::Text("pl-key".to_owned()));
    let Json::Object(posted) = field(body, "metadata") else {
        panic!("the posted metadata is a JSON object");
    };
    let set: IndexMap<String, Json> = posted
        .iter()
        .filter(|(_name, value)| !matches!(value, Json::Null))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    assert_eq!(
        set,
        IndexMap::from([
            (
                "user_api_key_user_id".to_owned(),
                Json::Text("user-1".to_owned())
            ),
            (
                "requester_metadata".to_owned(),
                Json::Object(IndexMap::from([(
                    "pl_tags".to_owned(),
                    Json::List(vec![
                        Json::Text("tag-1".to_owned()),
                        Json::Text("tag-2".to_owned())
                    ])
                )]))
            ),
        ])
    );
}

#[tokio::test]
async fn a_null_metadata_value_is_posted_as_null_and_pl_tags_still_split_off() {
    let capture = Capture::answering(SUCCESS_WITH_REQUEST);
    let adapter = adapter(capture.clone(), settings(Some("pl-key")));
    let with_null = event_from(
        &PAYLOAD.replace(
            r#""requester_metadata": {"pl_tags": ["tag-1", "tag-2"]}"#,
            r#""requester_metadata": {"pl_tags": ["tag-1", "tag-2"], "stage": null}"#,
        ),
        false,
    );
    deliver(&with_null, &adapter).await;
    let requests = capture.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        field(&requests[0].body, "tags"),
        &Json::List(vec![
            Json::Text("tag-1".to_owned()),
            Json::Text("tag-2".to_owned())
        ])
    );
    let metadata = field(&requests[1].body, "metadata");
    assert_eq!(field(metadata, "stage"), &Json::Null);
    assert_eq!(field(metadata, "user_api_key_org_id"), &Json::Null);
    let Json::Object(posted) = metadata else {
        panic!("the posted metadata is a JSON object");
    };
    assert!(!posted.contains_key("pl_tags"));
}

#[tokio::test]
async fn an_unsuccessful_response_sends_no_metadata() {
    let capture = Capture::answering(UNSUCCESSFUL);
    let adapter = adapter(capture.clone(), settings(Some("pl-key")));
    deliver(&event(false), &adapter).await;
    assert_eq!(capture.requests().len(), 1);
}

#[tokio::test]
async fn an_event_without_messages_posts_nothing() {
    let capture = Capture::answering(SUCCESS);
    let adapter = adapter(capture.clone(), settings(Some("pl-key")));
    let without_messages = event_from(
        &PAYLOAD.replace(
            "\n    \"messages\": [{\"role\": \"user\", \"content\": \"Hello\"}],",
            "",
        ),
        false,
    );
    deliver(&without_messages, &adapter).await;
    assert!(capture.requests().is_empty());
}

#[tokio::test]
async fn a_stream_chunk_event_sends_nothing() {
    let capture = Capture::answering(SUCCESS);
    let adapter = adapter(capture.clone(), settings(Some("pl-key")));
    adapter
        .log_stream_event(&event(true))
        .expect("the chunk is accepted");
    adapter.tracker.close();
    adapter.tracker.wait().await;
    assert!(capture.requests().is_empty());
}

#[test]
fn an_invalid_start_time_is_reported() {
    let capture = Capture::answering(SUCCESS);
    let adapter = adapter(capture, settings(Some("pl-key")));
    let broken = event_from(
        &PAYLOAD.replace("1704067200.0", "100000000000000000000"),
        false,
    );
    let error = adapter
        .log_success_event(&broken)
        .expect_err("the event is rejected");
    assert!(matches!(error, LoggerError::Delivery { callback, .. } if callback == "promptlayer"));
}
