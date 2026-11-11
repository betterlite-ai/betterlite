#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
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

use super::adapter::GalileoAdapter;
use crate::callback_logger::CallbackLogger as _;
use crate::callback_logger::event::LoggerEvent;
use crate::galileo::delivery::config::GalileoConfig;
use crate::galileo::delivery::headers::Header;
use crate::galileo::delivery::transport::{HttpResponse, TraceIds, Transport, TransportError};
use crate::galileo::delivery::{GalileoDelivery, State};
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
    let trace = trace_of(payload);
    assert_eq!(field(&trace, "type"), &Json::Text("trace".to_owned()));
    assert_eq!(field(&trace, "id"), &Json::Text("id-0".to_owned()));
    assert_eq!(field(&trace, "name"), &Json::Text("completion".to_owned()));
    assert_eq!(field(&trace, "input"), &Json::Text("Hello".to_owned()));
    assert_eq!(
        field(&trace, "created_at"),
        &Json::Text("2024-01-01T00:00:00Z".to_owned())
    );
    let Json::List(spans) = field(&trace, "spans") else {
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
