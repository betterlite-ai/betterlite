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

use super::adapter::LiteralAiAdapter;
use crate::batch::logger::BatchQueue;
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};
use crate::literal_ai::literal_ai_logging::LiteralAILogger;
use crate::literal_ai::literal_ai_logging::ids::SystemIds;
use crate::literal_ai::literal_ai_logging::settings::{Overrides, Settings};
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
    "custom_llm_provider": "openai",
    "api_base": "https://api.openai.com",
    "metadata": {
        "user_api_key_user_id": "user-1",
        "requester_metadata": {
            "step_id": "step-1",
            "literalai_thread_id": "thread-1",
            "literalai_tags": ["tier", "prod"]
        }
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

fn settings(batch_size: &str) -> Settings {
    Settings::from_environment(
        &BTreeMap::from([
            ("LITERAL_API_KEY".to_owned(), "literal-key".to_owned()),
            ("LITERAL_BATCH_SIZE".to_owned(), batch_size.to_owned()),
        ]),
        Overrides::default(),
    )
    .expect("the settings resolve")
}

fn fixture() -> (LiteralAiAdapter<Captured, SystemIds>, Captured, TaskTracker) {
    let capture = Captured::default();
    let tracker = TaskTracker::new();
    let settings = settings("1");
    let adapter = LiteralAiAdapter {
        logger: Arc::new(LiteralAILogger {
            queue: BatchQueue::new(settings.batch),
            settings,
            delivery: capture.clone(),
            ids: SystemIds,
        }),
        tracker: tracker.clone(),
    };
    (adapter, capture, tracker)
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

fn variables(body: &Json) -> &IndexMap<String, Json> {
    let Json::Object(fields) = body else {
        panic!("the posted body is an object");
    };
    let Some(Json::Object(variables)) = fields.get("variables") else {
        panic!("the posted body carries the variables");
    };
    variables
}

fn field<'body>(body: &'body Json, name: &str) -> &'body Json {
    variables(body)
        .get(name)
        .unwrap_or_else(|| panic!("the variables carry {name}"))
}

fn assert_request(requests: &[Recorded]) {
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "https://cloud.getliteral.ai/api/graphql");
    assert_eq!(
        requests[0].headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        requests[0].headers.get("x-client-name").map(String::as_str),
        Some("litellm")
    );
    assert_eq!(
        requests[0].headers.get("x-api-key").map(String::as_str),
        Some("literal-key")
    );
}

fn assert_step_fields(body: &Json) {
    assert_eq!(field(body, "id_0"), &Json::Text("step-1".to_owned()));
    assert_eq!(field(body, "name_0"), &Json::Text("gpt-4o".to_owned()));
    assert_eq!(field(body, "type_0"), &Json::Text("llm".to_owned()));
    assert_eq!(
        field(body, "threadId_0"),
        &Json::Text("thread-1".to_owned())
    );
    assert_eq!(field(body, "tags_0"), &json(r#"["tier", "prod"]"#));
    assert_eq!(
        member(field(body, "metadata_0"), "user_api_key_user_id"),
        &Json::Text("user-1".to_owned())
    );
    assert_eq!(
        field(body, "startTime_0"),
        &Json::Text("2024-01-01T00:00:00.000000Z".to_owned())
    );
    assert_eq!(
        field(body, "endTime_0"),
        &Json::Text("2024-01-01T00:00:01.500000Z".to_owned())
    );
}

fn assert_generation_fields(body: &Json) {
    let generation = field(body, "generation_0");
    assert_eq!(member(generation, "inputTokenCount"), &json("2"));
    assert_eq!(member(generation, "outputTokenCount"), &json("3"));
    assert_eq!(member(generation, "tokenCount"), &json("5"));
    assert_eq!(
        member(generation, "provider"),
        &Json::Text("openai".to_owned())
    );
    assert_eq!(
        member(generation, "model"),
        &Json::Text("gpt-4o".to_owned())
    );
    assert_eq!(member(generation, "duration"), &json("1.5"));
    assert_eq!(
        member(generation, "settings"),
        &json(r#"{"temperature": 0.2}"#)
    );
    assert_eq!(
        member(member(generation, "messageCompletion"), "content"),
        &Json::Text("hi".to_owned())
    );
    let Json::List(messages) = member(generation, "messages") else {
        panic!("the generation carries the messages list");
    };
    assert_eq!(
        member(&messages[0], "content"),
        &Json::Text("Hello".to_owned())
    );
}

#[test]
fn resolves_by_name_to_a_literalai_logger() {
    let catalog = LoggerCatalog::new(inputs(&[("LITERAL_API_KEY", "literal-key")]));
    let logger = catalog
        .resolve_logger("literalai")
        .expect("the name is available")
        .expect("the logger builds");
    assert_eq!(logger.callback_name(), "literalai");
}

#[test]
fn the_logger_still_builds_without_an_api_key() {
    let catalog = LoggerCatalog::new(inputs(&[]));
    let logger = catalog
        .resolve_logger("literalai")
        .expect("the name is available")
        .expect("LiteLLM builds LiteralAILogger without a key");
    assert_eq!(logger.callback_name(), "literalai");
}

#[test]
fn the_catalog_environment_reaches_the_settings() {
    let inputs = inputs(&[
        ("LITERAL_API_KEY", "literal-key"),
        ("LITERAL_API_URL", "https://literal.internal"),
    ]);
    let settings = Settings::from_environment(
        &super::environment(inputs.env.as_ref()),
        Overrides::default(),
    )
    .expect("the settings resolve");
    assert_eq!(settings.api_url, "https://literal.internal");
    assert_eq!(
        settings.api_key.map(|key| key.expose_secret().to_owned()),
        Some("literal-key".to_owned())
    );
}

#[tokio::test]
async fn a_success_event_posts_the_mapped_step() {
    let (adapter, capture, tracker) = fixture();
    adapter
        .log_success_event(&event())
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_request(&requests);
    assert_step_fields(&requests[0].body);
    assert_generation_fields(&requests[0].body);
}

#[tokio::test]
async fn the_request_tags_supply_the_step_tags() {
    let (adapter, capture, tracker) = fixture();
    let with_request_tags =
        event_from(&PAYLOAD.replace(r#""request_tags": []"#, r#""request_tags": ["prod"]"#));
    adapter
        .log_success_event(&with_request_tags)
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_request(&requests);
    assert_eq!(field(&requests[0].body, "tags_0"), &json(r#"["prod"]"#));
}

#[tokio::test]
async fn the_user_agent_tags_are_not_step_tags() {
    let (adapter, capture, tracker) = fixture();
    let with_user_agent = event_from(&PAYLOAD.replace(
        r#""request_tags": []"#,
        r#""request_tags": ["User-Agent: curl", "User-Agent: curl/8.5"]"#,
    ));
    adapter
        .log_success_event(&with_user_agent)
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_request(&requests);
    assert_eq!(
        field(&requests[0].body, "tags_0"),
        &json(r#"["tier", "prod"]"#)
    );
}

#[tokio::test]
async fn a_metadata_tag_that_names_a_header_is_kept() {
    let (adapter, capture, tracker) = fixture();
    let with_named_tag = event_from(&PAYLOAD.replace(
        r#""request_tags": []"#,
        r#""request_tags": ["team: search"]"#,
    ));
    adapter
        .log_success_event(&with_named_tag)
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_request(&requests);
    assert_eq!(
        field(&requests[0].body, "tags_0"),
        &json(r#"["team: search"]"#)
    );
}

#[tokio::test]
async fn the_requester_tags_win_over_the_request_tags() {
    let (adapter, capture, tracker) = fixture();
    let with_own_tags = event_from(
        &PAYLOAD
            .replace(
                r#""literalai_tags": ["tier", "prod"]"#,
                r#""tags": ["own"]"#,
            )
            .replace(r#""request_tags": []"#, r#""request_tags": ["prod"]"#),
    );
    adapter
        .log_success_event(&with_own_tags)
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_request(&requests);
    assert_eq!(field(&requests[0].body, "tags_0"), &json(r#"["own"]"#));
}

#[tokio::test]
async fn a_failure_event_posts_the_error() {
    let (adapter, capture, tracker) = fixture();
    let failed =
        event_from(&PAYLOAD.replace(r#""error_str": null"#, r#""error_str": "upstream said no""#));
    adapter
        .log_failure_event(&failed)
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let requests = capture.requests();
    assert_request(&requests);
    assert_eq!(
        field(&requests[0].body, "error_0"),
        &Json::Text("upstream said no".to_owned())
    );
}

#[tokio::test]
async fn an_async_success_event_flushes_the_queue() {
    let (adapter, capture, _tracker) = fixture();
    adapter
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    let requests = capture.requests();
    assert_request(&requests);
    assert_step_fields(&requests[0].body);
}

#[tokio::test]
async fn an_async_failure_event_flushes_the_queue() {
    let (adapter, capture, _tracker) = fixture();
    let failed =
        event_from(&PAYLOAD.replace(r#""error_str": null"#, r#""error_str": "upstream said no""#));
    adapter
        .async_log_failure_event(Arc::new(failed))
        .await
        .expect("the event is accepted");
    let requests = capture.requests();
    assert_request(&requests);
    assert_eq!(
        field(&requests[0].body, "error_0"),
        &Json::Text("upstream said no".to_owned())
    );
}

#[test]
fn an_invalid_start_time_is_reported() {
    let (adapter, capture, _tracker) = fixture();
    let broken = event_from(&PAYLOAD.replace("1704067200.0", "100000000000000000000"));
    let error = adapter
        .log_success_event(&broken)
        .expect_err("the event is rejected");
    assert!(matches!(error, LoggerError::Delivery { callback, .. } if callback == "literalai"));
    assert!(capture.requests().is_empty());
}
