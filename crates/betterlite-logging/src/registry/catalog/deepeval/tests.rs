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
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::adapter::DeepevalAdapter;
use super::build;
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::deepeval::deepeval_logging::DeepEvalLogger;
use crate::deepeval::deepeval_logging::ids::SystemUuids;
use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};
use crate::registry::catalog::{CatalogError, CatalogInputs, LoggerCatalog};

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
    "metadata": {"user_api_key_user_id": "user-1"},
    "cache_hit": false,
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "Hello"}],
    "response": {
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}}],
        "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}
    },
    "error_str": null
}"#;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Recorded {
    url: String,
    headers: BTreeMap<String, String>,
    body: Json,
}

#[derive(Clone)]
struct Captured {
    status: u16,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl Captured {
    fn answering(status: u16) -> Self {
        Self {
            status,
            requests: Arc::default(),
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
            status: self.status,
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

fn fixture(
    status: u16,
) -> (
    DeepevalAdapter<Captured, SystemUuids>,
    Captured,
    TaskTracker,
) {
    let capture = Captured::answering(status);
    let tracker = TaskTracker::new();
    let environment =
        BTreeMap::from([("CONFIDENT_API_KEY".to_owned(), "confident-key".to_owned())]);
    let logger = DeepEvalLogger::from_environment(&environment, capture.clone(), SystemUuids)
        .expect("the settings resolve");
    (
        DeepevalAdapter {
            logger: Arc::new(logger),
            tracker: tracker.clone(),
        },
        capture,
        tracker,
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

fn failure_event() -> LoggerEvent {
    event_from(&PAYLOAD.replace(
        r#""error_str": null"#,
        r#""error_str": "upstream returned 500""#,
    ))
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture JSON decodes")
}

fn member<'value>(value: &'value Json, name: &str) -> &'value Json {
    let Json::Object(fields) = value else {
        panic!("the value is an object");
    };
    fields
        .get(name)
        .unwrap_or_else(|| panic!("the object carries {name}"))
}

fn llm_span(body: &Json) -> &Json {
    let Json::List(spans) = member(body, "llmSpans") else {
        panic!("the trace carries llmSpans");
    };
    &spans[0]
}

fn assert_request(requests: &[Recorded]) {
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "https://api.confident-ai.com/v1/tracing");
    assert_eq!(
        requests[0].headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        requests[0]
            .headers
            .get("confident_api_key")
            .map(String::as_str),
        Some("confident-key")
    );
}

#[test]
fn resolves_by_name_to_a_deepeval_logger() {
    let catalog = LoggerCatalog::new(inputs(&[("CONFIDENT_API_KEY", "confident-key")]));
    let logger = catalog
        .resolve_logger("deepeval")
        .expect("the name is available")
        .expect("the logger builds");
    assert_eq!(logger.callback_name(), "deepeval");
}

#[test]
fn a_missing_api_key_gives_lite_llms_init_message() {
    let error = build(&inputs(&[])).err().expect("the key is required");
    assert!(matches!(
        error,
        CatalogError::Init { message }
            if message == "Please set 'CONFIDENT_API_KEY=<>' in your environment variables."
    ));
}

#[test]
fn an_invalid_environment_gives_lite_llms_init_message() {
    let error = build(&inputs(&[
        ("CONFIDENT_API_KEY", "confident-key"),
        ("LITELM_ENVIRONMENT", "qa"),
    ]))
    .err()
    .expect("the environment is validated");
    assert!(matches!(
        error,
        CatalogError::Init { message }
            if message
                == "Invalid environment: qa. Please use one of the following instead: \"production\", \"development\", \"staging\""
    ));
}

#[tokio::test]
async fn a_success_event_posts_the_mapped_trace() {
    let (adapter, capture, tracker) = fixture(200);
    adapter
        .log_success_event(&event())
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_request(&requests);
    let body = &requests[0].body;
    assert_eq!(member(body, "uuid"), &Json::Text("trace-1".to_owned()));
    assert_eq!(
        member(body, "environment"),
        &Json::Text("development".to_owned())
    );
    let span = llm_span(body);
    assert_eq!(member(span, "uuid"), &Json::Text("chatcmpl-1".to_owned()));
    assert_eq!(
        member(span, "name"),
        &Json::Text("litellm_success_callback".to_owned())
    );
    assert_eq!(member(span, "status"), &Json::Text("SUCCESS".to_owned()));
    assert_eq!(member(span, "type"), &Json::Text("llm".to_owned()));
    assert_eq!(member(span, "traceUuid"), &Json::Text("trace-1".to_owned()));
    assert_eq!(
        member(span, "startTime"),
        &Json::Text("2024-01-01T00:00:00.000Z".to_owned())
    );
    assert_eq!(
        member(span, "endTime"),
        &Json::Text("2024-01-01T00:00:01.500Z".to_owned())
    );
    assert_eq!(
        member(span, "input"),
        &json(r#"[{"role": "user", "content": "Hello"}]"#)
    );
    assert_eq!(member(span, "output"), &Json::Text("hi".to_owned()));
    assert_eq!(member(span, "model"), &Json::Text("gpt-4o".to_owned()));
    assert_eq!(member(span, "inputTokenCount"), &json("2"));
    assert_eq!(member(span, "outputTokenCount"), &json("3"));
}

#[tokio::test]
async fn a_failure_event_posts_the_error_as_the_output() {
    let (adapter, capture, tracker) = fixture(200);
    adapter
        .log_failure_event(&failure_event())
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_request(&requests);
    let span = llm_span(&requests[0].body);
    assert_eq!(
        member(span, "name"),
        &Json::Text("litellm_failure_callback".to_owned())
    );
    assert_eq!(member(span, "status"), &Json::Text("ERRORED".to_owned()));
    assert_eq!(
        member(span, "input"),
        &json(r#"[{"role": "user", "content": "Hello"}]"#)
    );
    assert_eq!(
        member(span, "output"),
        &Json::Text("upstream returned 500".to_owned())
    );
    assert_eq!(
        member(&requests[0].body, "environment"),
        &Json::Text("development".to_owned())
    );
}

#[tokio::test]
async fn an_async_success_event_posts_the_mapped_trace() {
    let (adapter, capture, _tracker) = fixture(200);
    adapter
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    let requests = capture.requests();
    assert_request(&requests);
    assert_eq!(
        member(llm_span(&requests[0].body), "output"),
        &Json::Text("hi".to_owned())
    );
}

#[tokio::test]
async fn an_async_failure_event_reports_a_rejected_trace() {
    let (adapter, capture, _tracker) = fixture(500);
    let error = adapter
        .async_log_failure_event(Arc::new(failure_event()))
        .await
        .expect_err("the rejection is reported");
    assert!(matches!(
        error,
        LoggerError::Delivery { callback, message }
            if callback == "deepeval" && message.contains("500")
    ));
    assert_eq!(capture.requests().len(), 1);
}
