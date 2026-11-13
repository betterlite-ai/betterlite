#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::adapter::BraintrustAdapter;
use super::build;
use crate::braintrust_logging::logger::BraintrustLogger;
use crate::braintrust_logging::settings::Settings;
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};
use crate::registry::catalog::{CatalogError, CatalogInputs, LoggerCatalog};

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "trace-1",
    "session_id": "session-1",
    "litellm_call_id": "call-1",
    "call_type": "__CALL_TYPE__",
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
        "requester_metadata": __REQUESTER__
    },
    "cache_hit": false,
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "Hello"}],
    "response": __RESPONSE__,
    "error_str": null
}"#;

const CHAT_RESPONSE: &str = r#"{
    "id": "chatcmpl-1",
    "object": "chat.completion",
    "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}}],
    "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}
}"#;

const TEXT_RESPONSE: &str = r#"{
    "id": "cmpl-1",
    "object": "text_completion",
    "choices": [{"index": 0, "text": "hi"}],
    "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}
}"#;

const EMBEDDING_RESPONSE: &str = r#"{
    "object": "list",
    "data": [{"object": "embedding", "index": 0, "embedding": [0.25]}],
    "usage": {"prompt_tokens": 2, "total_tokens": 2}
}"#;

const IMAGE_RESPONSE: &str = r#"{
    "created": 1704067200,
    "data": [{"url": "https://images.example/one.png"}]
}"#;

const REQUESTER: &str = r#"{
    "project_id": "proj-1",
    "span_name": "My Span",
    "environment": "prod",
    "span_parents": "a, b"
}"#;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Recorded {
    url: String,
    headers: BTreeMap<String, String>,
    body: Json,
}

#[derive(Clone, Default)]
struct Captured {
    replies: Arc<Mutex<VecDeque<String>>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl Captured {
    fn with_replies<I>(replies: I) -> Self
    where
        I: IntoIterator<Item = &'static str>,
    {
        Self {
            replies: Arc::new(Mutex::new(replies.into_iter().map(str::to_owned).collect())),
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
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| "null".to_owned());
        ready(Ok(PostResponse {
            status: 200,
            body: reply.into_bytes(),
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

fn fixture(replies: &[&'static str]) -> (BraintrustAdapter<Captured>, Captured, TaskTracker) {
    let capture = Captured::with_replies(replies.iter().copied());
    let tracker = TaskTracker::new();
    let settings = Settings {
        api_key: SecretString::from("braintrust-key"),
        api_base: "https://api.braintrustdata.com/v1".to_owned(),
        default_tags: Vec::new(),
    };
    let logger = BraintrustLogger::new(settings, capture.clone());
    let adapter = BraintrustAdapter {
        logger: Arc::new(logger),
        tracker: tracker.clone(),
    };
    (adapter, capture, tracker)
}

fn event_of(payload: &str) -> LoggerEvent {
    LoggerEvent {
        payload: Arc::new(serde_json::from_str(payload).expect("the fixture payload decodes")),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    }
}

fn event_with(requester: &str, call_type: &str, response: &str) -> LoggerEvent {
    event_of(
        &PAYLOAD
            .replace("__REQUESTER__", requester)
            .replace("__CALL_TYPE__", call_type)
            .replace("__RESPONSE__", response),
    )
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

fn posted_event(body: &Json) -> &Json {
    let Json::List(events) = member(body, "events") else {
        panic!("the body carries the events list");
    };
    &events[0]
}

#[test]
fn resolves_by_name_to_a_braintrust_logger() {
    let catalog = LoggerCatalog::new(inputs(&[("BRAINTRUST_API_KEY", "braintrust-key")]));
    let logger = catalog
        .resolve_logger("braintrust")
        .expect("the name is available")
        .expect("the logger builds");
    assert_eq!(logger.callback_name(), "braintrust");
}

#[test]
fn a_missing_api_key_gives_lite_llms_init_message() {
    let error = build(&inputs(&[])).err().expect("the key is required");
    assert!(matches!(
        error,
        CatalogError::Init { message }
            if message == "Missing keys=['BRAINTRUST_API_KEY'] in environment."
    ));
}

fn assert_insert_request(requests: &[Recorded], project: &str) {
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        format!("https://api.braintrustdata.com/v1/project_logs/{project}/insert")
    );
    assert_eq!(
        requests[0].headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        requests[0].headers.get("authorization").map(String::as_str),
        Some("Bearer braintrust-key")
    );
}

fn assert_event_fields(event: &Json) {
    assert_eq!(member(event, "id"), &Json::Text("call-1".to_owned()));
    assert_eq!(
        member(event, "input"),
        &json(r#"[{"role": "user", "content": "Hello"}]"#)
    );
    assert_eq!(
        member(event, "span_attributes"),
        &json(r#"{"name": "My Span", "type": "llm"}"#)
    );
    assert_eq!(member(event, "span_parents"), &json(r#"["a", "b"]"#));
    assert_eq!(member(event, "tags"), &json("[]"));
    let metadata = member(event, "metadata");
    // the request metadata is merged flat into the standard logging object, as LiteLLM hands it to braintrust
    assert_eq!(
        member(metadata, "environment"),
        &Json::Text("prod".to_owned())
    );
    assert_eq!(member(metadata, "model"), &Json::Text("gpt-4o".to_owned()));
    let Json::List(output) = member(event, "output") else {
        panic!("the event carries the output choices");
    };
    assert_eq!(
        member(&output[0], "finish_reason"),
        &Json::Text("stop".to_owned())
    );
    assert_eq!(
        member(member(&output[0], "message"), "content"),
        &Json::Text("hi".to_owned())
    );
}

fn assert_metrics(event: &Json, first_token: Option<&str>) {
    let metrics = member(event, "metrics");
    assert_eq!(member(metrics, "prompt_tokens"), &json("2"));
    assert_eq!(member(metrics, "completion_tokens"), &json("3"));
    assert_eq!(member(metrics, "total_tokens"), &json("5"));
    assert_eq!(member(metrics, "total_cost"), &json("0.5"));
    if let Some(value) = first_token {
        assert_eq!(member(metrics, "time_to_first_token"), &json(value));
        return;
    }
    let Json::Object(fields) = metrics else {
        panic!("the metrics are an object");
    };
    assert!(!fields.contains_key("time_to_first_token"));
}

#[tokio::test]
async fn a_success_event_posts_the_mapped_event() {
    let (adapter, capture, tracker) = fixture(&[]);
    adapter
        .log_success_event(&event_with(REQUESTER, "completion", CHAT_RESPONSE))
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_insert_request(&requests, "proj-1");
    let event = posted_event(&requests[0].body);
    assert_event_fields(event);
    assert_metrics(event, Some("1.5"));
}

#[tokio::test]
async fn a_text_completion_event_posts_text_choices() {
    let (adapter, capture, tracker) = fixture(&[]);
    adapter
        .log_success_event(&event_with(REQUESTER, "text_completion", TEXT_RESPONSE))
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_insert_request(&requests, "proj-1");
    let event = posted_event(&requests[0].body);
    let Json::List(output) = member(event, "output") else {
        panic!("the event carries the output choices");
    };
    assert_eq!(member(&output[0], "text"), &Json::Text("hi".to_owned()));
    // a text choice with no finish reason keeps LiteLLM's null instead of the chat "stop"
    assert_eq!(member(&output[0], "finish_reason"), &Json::Null);
    assert!(matches!(member(event, "metrics"), Json::Object(_)));
}

#[tokio::test]
async fn an_embedding_event_posts_an_empty_output_with_metrics() {
    let (adapter, capture, tracker) = fixture(&[]);
    adapter
        .log_success_event(&event_with(REQUESTER, "embedding", EMBEDDING_RESPONSE))
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_insert_request(&requests, "proj-1");
    let event = posted_event(&requests[0].body);
    assert_eq!(member(event, "output"), &json("[]"));
    assert_eq!(
        member(member(event, "metrics"), "prompt_tokens"),
        &json("2")
    );
}

#[tokio::test]
async fn an_image_event_posts_an_empty_output_without_metrics() {
    let (adapter, capture, tracker) = fixture(&[]);
    adapter
        .log_success_event(&event_with(REQUESTER, "image_generation", IMAGE_RESPONSE))
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_insert_request(&requests, "proj-1");
    let event = posted_event(&requests[0].body);
    assert_eq!(member(event, "output"), &json("[]"));
    let Json::Object(fields) = event else {
        panic!("the event is an object");
    };
    assert!(!fields.contains_key("metrics"));
}

#[tokio::test]
async fn an_async_success_event_registers_the_project_it_names() {
    let requester = r#"{"project_name": "team", "span_name": "My Span"}"#;
    let (adapter, capture, _tracker) = fixture(&[r#"{"id": "team-1"}"#]);
    adapter
        .async_log_success_event(Arc::new(event_with(requester, "completion", CHAT_RESPONSE)))
        .await
        .expect("the event is accepted");
    let requests = capture.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].url,
        "https://api.braintrustdata.com/v1/project/register"
    );
    assert_eq!(requests[0].body, json(r#"{"name": "team"}"#));
    assert_eq!(
        requests[1].url,
        "https://api.braintrustdata.com/v1/project_logs/team-1/insert"
    );
    let event = posted_event(&requests[1].body);
    assert_eq!(
        member(event, "span_attributes"),
        &json(r#"{"name": "My Span", "type": "llm"}"#)
    );
    // the payload keeps no provider-handoff instant, so the async path leaves time to first token unset
    assert_metrics(event, None);
}

#[test]
fn an_invalid_start_time_is_reported() {
    let (adapter, capture, _tracker) = fixture(&[]);
    let broken = PAYLOAD
        .replace("1704067200.0", "100000000000000000000")
        .replace("__REQUESTER__", REQUESTER)
        .replace("__CALL_TYPE__", "completion")
        .replace("__RESPONSE__", CHAT_RESPONSE);
    let error = adapter
        .log_success_event(&event_of(&broken))
        .expect_err("the event is rejected");
    assert!(matches!(error, LoggerError::Delivery { callback, .. } if callback == "braintrust"));
    assert!(capture.requests().is_empty());
}

#[tokio::test]
async fn a_cost_failure_posts_a_null_total_cost() {
    let (adapter, capture, tracker) = fixture(&[]);
    let event = event_of(
        &PAYLOAD
            .replace(
                r#""response_cost": 0.5,"#,
                r#""response_cost": 0.5, "response_cost_failure_debug_info": {"error": "no price"},"#,
            )
            .replace("__REQUESTER__", REQUESTER)
            .replace("__CALL_TYPE__", "completion")
            .replace("__RESPONSE__", CHAT_RESPONSE),
    );
    adapter
        .log_success_event(&event)
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_insert_request(&requests, "proj-1");
    let posted = posted_event(&requests[0].body);
    assert_eq!(member(member(posted, "metrics"), "total_cost"), &Json::Null);
}

#[tokio::test]
async fn a_guardrail_cost_stays_out_of_the_total_cost() {
    let (adapter, capture, tracker) = fixture(&[]);
    let event = event_of(
        &PAYLOAD
            .replace(
                r#""response_cost": 0.5,"#,
                r#""response_cost": 0.75, "cost_breakdown": {"total_cost": 0.75, "guardrail_cost": 0.25},"#,
            )
            .replace("__REQUESTER__", REQUESTER)
            .replace("__CALL_TYPE__", "completion")
            .replace("__RESPONSE__", CHAT_RESPONSE),
    );
    adapter
        .log_success_event(&event)
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_insert_request(&requests, "proj-1");
    let posted = posted_event(&requests[0].body);
    assert_eq!(
        member(member(posted, "metrics"), "total_cost"),
        &json("0.5")
    );
}
