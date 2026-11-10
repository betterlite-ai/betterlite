#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry::Value;
use opentelemetry::trace::{SpanId, TraceId, TracerProvider as _};
use opentelemetry_sdk::trace::{
    InMemorySpanExporter, RandomIdGenerator, SdkTracer, SdkTracerProvider, Span as SdkSpan,
    SpanData,
};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio_util::task::TaskTracker;

use crate::fields::payload::StandardLoggingPayload;
use crate::langfuse_child_observations::child_spans::ChildSpans;
use crate::langfuse_child_observations::prompts::{GenerationPrompt, PromptManagementMetadata};
use crate::langfuse_observations::requested_ids::RequestedIdGenerator;
use crate::langfuse_observations::span::{GenerationStart, LangfuseObservation, start_generation};
use crate::langfuse_rest::cache::Clock;
use crate::langfuse_rest::client::{
    LangfuseApiClient, LangfuseClientConfig, build_langfuse_client,
};
use crate::langfuse_rest::records::Prompt;

const TRACE_ID: &str = "0102030405060708090a0b0c0d0e0f10";
const GENERATION_ID: &str = "0102030405060708";

const BASE_PAYLOAD: &str = r#"{
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
    "response_time": 1.0,
    "model_map_information": {"model_map_key": "", "model_map_value": null},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {}"#;

pub(crate) fn provider() -> (SdkTracerProvider, InMemorySpanExporter) {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .with_id_generator(RequestedIdGenerator::default())
        .build();
    (provider, exporter)
}

pub(crate) fn sdk_tracer(provider: &SdkTracerProvider) -> SdkTracer {
    provider.tracer("langfuse-child-observations")
}

pub(crate) fn span(text: &str) -> SpanId {
    SpanId::from_hex(text).expect("a span id")
}

pub(crate) fn generation(tracer: &SdkTracer) -> LangfuseObservation<SdkSpan> {
    start_generation(
        tracer,
        &RandomIdGenerator::default(),
        GenerationStart {
            trace_id: TraceId::from_hex(TRACE_ID).expect("a trace id"),
            parent_observation_id: None,
            existing_trace: false,
            observation_id: Some(span(GENERATION_ID)),
            name: "generation".to_owned(),
            start_time: None,
            public: None,
            attributes: Vec::new(),
        },
    )
}

pub(crate) fn child<'target>(
    tracer: &'target SdkTracer,
    parent: &'target LangfuseObservation<SdkSpan>,
) -> ChildSpans<'target, SdkTracer> {
    ChildSpans { tracer, parent }
}

pub(crate) fn payload(
    guardrail_information: Option<&str>,
) -> Result<StandardLoggingPayload, serde_json::Error> {
    let guardrails = guardrail_information.map_or_else(String::new, |fragment| {
        format!(r#","guardrail_information":{fragment}"#)
    });
    serde_json::from_str(&format!("{BASE_PAYLOAD}{guardrails}}}"))
}

pub(crate) fn spans(exporter: &InMemorySpanExporter) -> Vec<SpanData> {
    exporter.get_finished_spans().expect("the exported spans")
}

pub(crate) fn attribute(data: &SpanData, key: &str) -> Option<Value> {
    data.attributes
        .iter()
        .find(|pair| pair.key.as_str() == key)
        .map(|pair| pair.value.clone())
}

pub(crate) fn attribute_json(data: &SpanData, key: &str) -> Option<Json> {
    let Value::String(text) = attribute(data, key)? else {
        return None;
    };
    serde_json::from_str(text.as_ref()).ok()
}

pub(crate) fn prompt_value(text: &str) -> Json {
    serde_json::from_str(text).expect("a prompt fixture")
}

pub(crate) fn metadata(prompt: Json) -> IndexMap<String, Json> {
    let mut metadata = IndexMap::new();
    metadata.insert("prompt".to_owned(), prompt);
    metadata.insert("session_id".to_owned(), Json::Text("session-1".to_owned()));
    metadata
}

pub(crate) fn grounding_enrichments(grounding: Json) -> IndexMap<String, Json> {
    let mut hidden = IndexMap::new();
    hidden.insert("vertex_ai_grounding_metadata".to_owned(), grounding);
    let mut enrichments = IndexMap::new();
    enrichments.insert("hidden_params".to_owned(), Json::Object(hidden));
    enrichments
}

pub(crate) fn prompt_client(attachment: Option<GenerationPrompt>) -> Option<Prompt> {
    match attachment {
        Some(GenerationPrompt::Client(prompt)) => Some(*prompt),
        Some(GenerationPrompt::Raw(_)) | None => None,
    }
}

pub(crate) fn prompt_metadata(prompt_id: &str, integration: &str) -> PromptManagementMetadata {
    PromptManagementMetadata {
        prompt_id: prompt_id.to_owned(),
        prompt_variables: None,
        prompt_integration: integration.to_owned(),
    }
}

struct WallClock;

impl Clock for WallClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

pub(crate) fn client(base_url: String) -> LangfuseApiClient {
    let policy =
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("a proxy policy");
    let http = ProxiedClient::outbound(&Arc::new(policy)).expect("an HTTP client");
    build_langfuse_client(
        LangfuseClientConfig {
            public_key: Some("pk-lf-test".to_owned()),
            secret_key: Some(SecretString::from("sk-lf-test")),
            base_url,
            timeout: Duration::from_secs(5),
            prompt_cache_ttl: Duration::ZERO,
        },
        Some(http),
        Arc::new(WallClock),
        TaskTracker::new(),
    )
    .expect("a Langfuse client")
}

pub(crate) fn idle_client() -> LangfuseApiClient {
    client("http://127.0.0.1:1".to_owned())
}

pub(crate) struct FakeLangfuse {
    pub(crate) base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    _tasks: JoinSet<()>,
}

impl FakeLangfuse {
    pub(crate) fn request_lines(&self) -> Vec<String> {
        let heads = self
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        heads
            .iter()
            .filter_map(|head| head.lines().next().map(str::to_owned))
            .collect()
    }

    pub(crate) fn request_count(&self) -> usize {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

pub(crate) async fn prompt_server(status: u16, body: &'static str) -> FakeLangfuse {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a listener");
    let address = listener.local_addr().expect("the listener address");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        while let Ok((mut stream, _peer)) = listener.accept().await {
            let head = read_head(&mut stream).await;
            seen.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(head);
            answer(&mut stream, status, body).await;
        }
    });
    FakeLangfuse {
        base_url: format!("http://{address}"),
        requests,
        _tasks: tasks,
    }
}

async fn read_head(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0_u8; 1];
        if stream.read_exact(&mut byte).await.is_err() {
            break;
        }
        head.extend(byte);
    }
    String::from_utf8(head).expect("the request head is UTF-8")
}

async fn answer(stream: &mut TcpStream, status: u16, body: &str) {
    let head = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _head_written = stream.write_all(head.as_bytes()).await;
    let _body_written = stream.write_all(body.as_bytes()).await;
    let _closed = stream.shutdown().await;
}
