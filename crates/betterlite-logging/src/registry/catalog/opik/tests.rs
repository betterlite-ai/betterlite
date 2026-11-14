#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::{Future, ready};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::loader::{ConfigLoader, EnvironmentLookup};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tempfile::TempDir;
use tokio::task::yield_now;
use tokio::time::{advance, timeout};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::adapter::OpikAdapter;
use super::build;
use super::build_logger;
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};
use crate::opik::delivery::logger::{LoggerDependencies, OpikLogger};
use crate::opik::delivery::native::NoNativeClient;
use crate::opik::delivery::settings::{LoggerOptions, resolve_settings};
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
    "custom_llm_provider": "openai",
    "api_base": "https://api.openai.com",
    "metadata": {
        "user_api_key_user_id": "user-1",
        "requester_metadata": {"opik": {"project_name": "proj-1", "thread_id": "thread-1"}}
    },
    "cache_hit": false,
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "Hello"}],
    "response": {
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1704067200,
        "model": "gpt-4o",
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
            status: 204,
            body: Vec::new(),
        }))
    }
}

struct Home {
    variables: BTreeMap<String, String>,
    home: PathBuf,
}

impl EnvironmentLookup for Home {
    fn lookup(&self, name: &str) -> Option<String> {
        self.variables.get(name).cloned()
    }

    fn system_home(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }
}

fn inputs_from(env: Arc<dyn EnvironmentLookup + Send + Sync>) -> CatalogInputs {
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
        env,
        callback_settings: BTreeMap::new(),
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

fn inputs(environment: &[(&str, &str)]) -> CatalogInputs {
    let env = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<String, String>>();
    inputs_from(Arc::new(env))
}

fn fixture(batch_size: usize) -> (OpikAdapter<Captured>, Captured, TaskTracker) {
    let capture = Captured::default();
    let spawns = TaskTracker::new();
    let flushes = TaskTracker::new();
    let options = LoggerOptions {
        project_name: None,
        url: None,
        api_key: None,
        workspace: None,
        batch_size: Some(batch_size),
        flush_interval: None,
        max_queue_size: None,
    };
    let environment = BTreeMap::from([("OPIK_API_KEY".to_owned(), "opik-key".to_owned())]);
    let logger = OpikLogger::build(
        &options,
        &environment,
        None,
        LoggerDependencies {
            delivery: capture.clone(),
            native: &NoNativeClient,
            tasks: Some(&flushes),
        },
    )
    .expect("the logger builds");
    (
        OpikAdapter {
            logger,
            tracker: spawns.clone(),
        },
        capture,
        spawns,
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

fn batch_entry<'body>(body: &'body Json, field: &str) -> &'body Json {
    let Json::List(entries) = member(body, field) else {
        panic!("the batch carries {field}");
    };
    &entries[0]
}

fn assert_requests(requests: &[Recorded]) {
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].url,
        "https://www.comet.com/opik/api/v1/private/traces/batch"
    );
    assert_eq!(
        requests[1].url,
        "https://www.comet.com/opik/api/v1/private/spans/batch"
    );
    for request in requests {
        assert_eq!(
            request.headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        assert_eq!(
            request.headers.get("authorization").map(String::as_str),
            Some("opik-key")
        );
    }
}

fn assert_trace(trace: &Json) {
    assert_eq!(
        member(trace, "project_name"),
        &Json::Text("proj-1".to_owned())
    );
    assert_eq!(
        member(trace, "name"),
        &Json::Text("chat.completion".to_owned())
    );
    assert_eq!(
        member(trace, "thread_id"),
        &Json::Text("thread-1".to_owned())
    );
    assert_eq!(member(trace, "tags"), &json(r#"["openai"]"#));
    assert_eq!(
        member(trace, "start_time"),
        &Json::Text("2024-01-01T00:00:00.000Z".to_owned())
    );
    assert_eq!(
        member(trace, "end_time"),
        &Json::Text("2024-01-01T00:00:01.500Z".to_owned())
    );
    let Json::List(input) = member(trace, "input") else {
        panic!("the trace carries the messages list");
    };
    assert_eq!(
        member(&input[0], "content"),
        &Json::Text("Hello".to_owned())
    );
    let metadata = member(trace, "metadata");
    assert_eq!(
        member(metadata, "created_from"),
        &Json::Text("litellm".to_owned())
    );
    assert_eq!(member(metadata, "model"), &Json::Text("gpt-4o".to_owned()));
    assert_eq!(
        member(member(metadata, "cost"), "total_tokens"),
        &json("0.5")
    );
}

fn assert_span(span: &Json, trace: &Json) {
    assert_eq!(member(span, "type"), &Json::Text("llm".to_owned()));
    assert_eq!(member(span, "model"), &Json::Text("gpt-4o".to_owned()));
    assert_eq!(
        member(span, "name"),
        &Json::Text("gpt-4o_chat.completion_1704067200".to_owned())
    );
    assert_eq!(member(span, "trace_id"), member(trace, "id"));
    assert_eq!(member(span, "total_cost"), &json("0.5"));
    let usage = member(span, "usage");
    assert_eq!(member(usage, "prompt_tokens"), &json("2"));
    assert_eq!(member(usage, "completion_tokens"), &json("3"));
    assert_eq!(member(usage, "total_tokens"), &json("5"));
}

#[tokio::test]
async fn resolves_by_name_to_an_opik_logger() {
    let catalog = LoggerCatalog::new(inputs(&[("OPIK_API_KEY", "opik-key")]));
    let logger = catalog
        .resolve_logger("opik")
        .expect("the name is available")
        .expect("the logger builds");
    assert_eq!(logger.callback_name(), "opik");
}

#[tokio::test]
async fn the_logger_still_builds_without_an_api_key() {
    let catalog = LoggerCatalog::new(inputs(&[]));
    let logger = catalog
        .resolve_logger("opik")
        .expect("the name is available")
        .expect("LiteLLM builds OpikLogger without a key");
    assert_eq!(logger.callback_name(), "opik");
}

#[tokio::test(start_paused = true)]
async fn the_flush_loop_flushes_on_the_interval_and_stops_on_shutdown() {
    let inputs = inputs(&[
        ("OPIK_API_KEY", "opik-key"),
        ("DEFAULT_FLUSH_INTERVAL_SECONDS", "1"),
    ]);
    let capture = Captured::default();
    let logger = build_logger(&inputs, capture.clone()).expect("the logger builds");
    // the merged periodic flush and the shutdown watcher both run on the catalog's tracker
    assert_eq!(inputs.tracker.len(), 2);
    let adapter = OpikAdapter {
        logger,
        tracker: inputs.tracker.clone(),
    };
    yield_now().await;
    adapter
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    assert!(capture.requests().is_empty());
    advance(Duration::from_secs(1)).await;
    yield_now().await;
    assert_eq!(capture.requests().len(), 2);
    inputs.shutdown.cancel();
    yield_now().await;
    advance(Duration::from_secs(1)).await;
    yield_now().await;
    timeout(Duration::from_secs(1), inputs.tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
}

#[test]
fn the_catalog_environment_reaches_the_settings() {
    let inputs = inputs(&[("OPIK_API_KEY", "opik-key")]);
    let settings = resolve_settings(
        &LoggerOptions {
            project_name: None,
            url: None,
            api_key: None,
            workspace: None,
            batch_size: None,
            flush_interval: None,
            max_queue_size: None,
        },
        &super::environment(inputs.env.as_ref()),
        None,
    )
    .expect("the settings resolve");
    assert_eq!(
        settings
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("opik-key")
    );
}

#[tokio::test]
async fn the_config_file_is_read_when_the_environment_is_silent() {
    let temporary = TempDir::new().expect("a temporary home");
    fs::write(
        temporary.path().join(".opik.config"),
        "[opik]\nproject_name = 50%\n",
    )
    .expect("the fixture config file is written");
    let home = Home {
        variables: BTreeMap::new(),
        home: temporary.path().to_path_buf(),
    };
    let error = build(&inputs_from(Arc::new(home)))
        .err()
        .expect("the malformed config file reaches the settings");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("configuration file")
    ));
}

#[tokio::test]
async fn a_success_event_posts_the_mapped_trace_and_span() {
    let (adapter, capture, tracker) = fixture(512);
    adapter
        .log_success_event(&event())
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_requests(&requests);
    let trace = batch_entry(&requests[0].body, "traces");
    let span = batch_entry(&requests[1].body, "spans");
    assert_trace(trace);
    assert_span(span, trace);
}

#[tokio::test]
async fn a_cost_failure_posts_no_cost_and_no_debug_info() {
    let (adapter, capture, tracker) = fixture(512);
    let failed_cost = event_from(&PAYLOAD.replace(
        r#""response_cost": 0.5,"#,
        r#""response_cost": 0.5, "response_cost_failure_debug_info": {"error": "no price"},"#,
    ));
    adapter
        .log_success_event(&failed_cost)
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_requests(&requests);
    let trace = batch_entry(&requests[0].body, "traces");
    let metadata = member(trace, "metadata");
    assert_eq!(
        member(member(metadata, "cost"), "total_tokens"),
        &Json::Null
    );
    let Json::Object(fields) = metadata else {
        panic!("the trace metadata is an object");
    };
    assert!(!fields.contains_key("response_cost_failure_debug_info"));
    let Json::Object(span) = batch_entry(&requests[1].body, "spans") else {
        panic!("the span is an object");
    };
    // the sync path posts the optional fields as nulls, as LiteLLM's payload dict keeps them
    assert_eq!(span.get("total_cost"), Some(&Json::Null));
}

#[tokio::test]
async fn a_guardrail_cost_stays_out_of_the_posted_cost() {
    let (adapter, capture, tracker) = fixture(512);
    let guarded = event_from(&PAYLOAD.replace(
        r#""response_cost": 0.5,"#,
        r#""response_cost": 0.75, "cost_breakdown": {"total_cost": 0.75, "guardrail_cost": 0.25},"#,
    ));
    adapter
        .log_success_event(&guarded)
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_requests(&requests);
    let trace = batch_entry(&requests[0].body, "traces");
    assert_eq!(
        member(member(member(trace, "metadata"), "cost"), "total_tokens"),
        &json("0.5")
    );
    assert_eq!(
        member(batch_entry(&requests[1].body, "spans"), "total_cost"),
        &json("0.5")
    );
}

#[tokio::test]
async fn an_async_success_event_flushes_the_queue_at_the_batch_size() {
    let (adapter, capture, _tracker) = fixture(1);
    adapter
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    let requests = capture.requests();
    assert_requests(&requests);
    let trace = batch_entry(&requests[0].body, "traces");
    assert_trace(trace);
    assert_span(batch_entry(&requests[1].body, "spans"), trace);
}

#[tokio::test]
async fn an_invalid_start_time_is_reported() {
    let (adapter, capture, _tracker) = fixture(512);
    let broken = event_from(&PAYLOAD.replace("1704067200.0", "100000000000000000000"));
    let error = adapter
        .log_success_event(&broken)
        .expect_err("the event is rejected");
    assert!(matches!(error, LoggerError::Delivery { callback, .. } if callback == "opik"));
    assert!(capture.requests().is_empty());
}
