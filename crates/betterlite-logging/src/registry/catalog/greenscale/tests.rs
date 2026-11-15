#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::Lookup;
use super::adapter::GreenscaleAdapter;
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::http_analytics_loggers::clock::WallClock;
use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};
use crate::http_analytics_loggers::greenscale::{GreenscaleLogger, GreenscaleSettings};
use crate::registry::catalog::{CatalogInputs, LoggerCatalog};

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
        "requester_metadata": {
            "greenscale_project": "project-1",
            "greenscale_application": "application-1",
            "greenscale_region": "eu",
            "unrelated": "kept-out"
        }
    },
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "Hello"}],
    "response": {"id": "chatcmpl-1", "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}}
}"#;

#[derive(Clone, Default)]
struct Recorder {
    requests: Arc<Mutex<Vec<Json>>>,
}

impl Recorder {
    fn bodies(&self) -> Vec<Json> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the recording delivery failed")]
struct RecordError;

impl Delivery for Recorder {
    type Error = RecordError;

    fn post(
        &self,
        request: PostRequest,
    ) -> impl Future<Output = Result<PostResponse, Self::Error>> + Send {
        let Ok(body) = serde_json::from_slice::<Json>(&request.body) else {
            return ready(Err(RecordError));
        };
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(body);
        ready(Ok(PostResponse {
            status: 200,
            body: Vec::new(),
        }))
    }
}

struct FixedClock {
    now: DateTime<Utc>,
}

impl WallClock for FixedClock {
    fn now_utc(&self) -> DateTime<Utc> {
        self.now
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

fn settings(environment: &[(&str, &str)]) -> GreenscaleSettings {
    let values: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    GreenscaleSettings::from_environment(&values)
}

async fn captured_event(settings: GreenscaleSettings) -> Vec<Json> {
    let recorder = Recorder::default();
    let tracker = TaskTracker::new();
    let adapter = GreenscaleAdapter {
        logger: Arc::new(GreenscaleLogger {
            delivery: recorder.clone(),
            clock: FixedClock {
                now: DateTime::from_timestamp(1_767_322_800, 0).expect("a valid fixture time"),
            },
            settings,
        }),
        tracker: tracker.clone(),
    };
    adapter
        .log_success_event(&event())
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    recorder.bodies()
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
fn resolves_by_name_to_a_greenscale_logger() {
    let catalog = LoggerCatalog::new(inputs(&[
        ("GREENSCALE_API_KEY", "green-key"),
        ("GREENSCALE_ENDPOINT", "https://greenscale.example.com/log"),
    ]));
    let logger = catalog
        .resolve_logger("greenscale")
        .expect("the name is available")
        .expect("the logger builds");
    assert_eq!(logger.callback_name(), "greenscale");
}

#[test]
fn the_logger_still_builds_without_configuration() {
    let catalog = LoggerCatalog::new(inputs(&[]));
    let logger = catalog
        .resolve_logger("greenscale")
        .expect("the name is available")
        .expect("LiteLLM builds GreenscaleLogger without configuration");
    assert_eq!(logger.callback_name(), "greenscale");
}

#[tokio::test]
async fn a_success_event_posts_the_mapped_fields() {
    let bodies = captured_event(settings(&[
        ("GREENSCALE_API_KEY", "green-key"),
        ("GREENSCALE_ENDPOINT", "https://greenscale.example.com/log"),
    ]))
    .await;
    assert_eq!(bodies.len(), 1);
    let body = &bodies[0];
    assert_eq!(field(body, "modelId"), &Json::Text("gpt-4o".to_owned()));
    assert_eq!(field(body, "inputTokenCount"), &number(2));
    assert_eq!(field(body, "outputTokenCount"), &number(3));
    assert_eq!(
        field(body, "timestamp"),
        &Json::Text("2026-01-02T03:00:00Z".to_owned())
    );
    assert_eq!(field(body, "invocationLatency"), &number(1500));
    assert_eq!(field(body, "project"), &Json::Text("project-1".to_owned()));
    assert_eq!(
        field(body, "application"),
        &Json::Text("application-1".to_owned())
    );
    assert_eq!(
        field(body, "tags"),
        &Json::List(vec![Json::Object(IndexMap::from([
            ("key".to_owned(), Json::Text("region".to_owned())),
            ("value".to_owned(), Json::Text("eu".to_owned()))
        ]))])
    );
}

#[tokio::test]
async fn an_async_success_event_posts_the_mapped_fields_once() {
    let recorder = Recorder::default();
    let tracker = TaskTracker::new();
    let adapter = GreenscaleAdapter {
        logger: Arc::new(GreenscaleLogger {
            delivery: recorder.clone(),
            clock: FixedClock {
                now: DateTime::from_timestamp(1_767_322_800, 0).expect("a valid fixture time"),
            },
            settings: settings(&[
                ("GREENSCALE_API_KEY", "green-key"),
                ("GREENSCALE_ENDPOINT", "https://greenscale.example.com/log"),
            ]),
        }),
        tracker: tracker.clone(),
    };
    adapter
        .async_log_success_event(Arc::new(event()))
        .await
        .expect("the event is accepted");
    tracker.close();
    tracker.wait().await;
    let bodies = recorder.bodies();
    assert_eq!(bodies.len(), 1);
    assert_eq!(
        field(&bodies[0], "modelId"),
        &Json::Text("gpt-4o".to_owned())
    );
}

#[tokio::test]
async fn a_missing_endpoint_sends_nothing() {
    let bodies = captured_event(settings(&[("GREENSCALE_API_KEY", "green-key")])).await;
    assert!(bodies.is_empty());
}

#[test]
fn the_catalog_environment_reaches_the_settings() {
    let inputs = inputs(&[
        ("GREENSCALE_API_KEY", "green-key"),
        ("GREENSCALE_ENDPOINT", "https://greenscale.example.com/log"),
    ]);
    let settings = GreenscaleSettings::from_environment(&Lookup(inputs.env.as_ref()));
    assert_eq!(
        settings.api_key.map(|key| key.expose_secret().to_owned()),
        Some("green-key".to_owned())
    );
    assert_eq!(
        settings.endpoint.as_deref(),
        Some("https://greenscale.example.com/log")
    );
}

#[tokio::test]
async fn a_stream_chunk_event_sends_nothing() {
    let recorder = Recorder::default();
    let tracker = TaskTracker::new();
    let adapter = GreenscaleAdapter {
        logger: Arc::new(GreenscaleLogger {
            delivery: recorder.clone(),
            clock: FixedClock {
                now: DateTime::from_timestamp(1_767_322_800, 0).expect("a valid fixture time"),
            },
            settings: settings(&[("GREENSCALE_API_KEY", "green-key")]),
        }),
        tracker: tracker.clone(),
    };
    adapter
        .log_stream_event(&event())
        .expect("the chunk is accepted");
    tracker.close();
    tracker.wait().await;
    assert!(recorder.bodies().is_empty());
}

#[test]
fn an_invalid_start_time_is_reported() {
    let adapter = GreenscaleAdapter {
        logger: Arc::new(GreenscaleLogger {
            delivery: Recorder::default(),
            clock: FixedClock {
                now: DateTime::from_timestamp(1_767_322_800, 0).expect("a valid fixture time"),
            },
            settings: settings(&[("GREENSCALE_API_KEY", "green-key")]),
        }),
        tracker: TaskTracker::new(),
    };
    let broken = event_from(&PAYLOAD.replace("1704067200.0", "100000000000000000000"));
    let error = adapter
        .log_success_event(&broken)
        .expect_err("the event is rejected");
    assert!(matches!(error, LoggerError::Delivery { callback, .. } if callback == "greenscale"));
}
