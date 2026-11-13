#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry::trace::{TraceId, TracerProvider as _};
use opentelemetry::{Array, StringValue, Value};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use rust_decimal::Decimal;
use secrecy::SecretString;
use tokio_util::task::TaskTracker;

use crate::fields::payload::StandardLoggingPayload;
use crate::langfuse_assembly::{
    AssemblyRuntime, AssemblySettings, GenerationTrace, assemble_generation_trace,
};
use crate::langfuse_observations::identity::{Timestamp, resolve_trace_id};
use crate::langfuse_observations::requested_ids::RequestedIdGenerator;
use crate::langfuse_rest::cache::Clock;
use crate::langfuse_rest::client::{
    LangfuseApiClient, LangfuseClientConfig, build_langfuse_client,
};

const PAYLOAD_TEMPLATE: &str = r#"{
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
    "metadata": %METADATA%,
    "saved_cache_cost": 0.0,
    "request_tags": ["team-a"],
    "model_parameters": {},
    "hidden_params": %HIDDEN%,
    "guardrail_information": %GUARDRAILS%
}"#;

pub fn value(text: &str) -> Json {
    serde_json::from_str(text).expect("valid fixture JSON")
}

pub fn string_array(items: &[&str]) -> Value {
    Value::Array(Array::String(
        items
            .iter()
            .map(|item| StringValue::from((*item).to_owned()))
            .collect(),
    ))
}

pub fn payload(metadata: &str) -> StandardLoggingPayload {
    payload_with(metadata, r#"{"cache_key": "hidden-1"}"#, "null")
}

pub fn payload_with(
    metadata: &str,
    hidden_params: &str,
    guardrails: &str,
) -> StandardLoggingPayload {
    let text = PAYLOAD_TEMPLATE
        .replace("%METADATA%", metadata)
        .replace("%HIDDEN%", hidden_params)
        .replace("%GUARDRAILS%", guardrails);
    serde_json::from_str(&text).expect("a valid standard logging payload")
}

pub fn trace_id(text: &str) -> TraceId {
    TraceId::from_hex(text).expect("a trace id")
}

pub fn hashed(text: &str) -> TraceId {
    resolve_trace_id(Some(text), &RequestedIdGenerator::default())
}

pub fn run<F>(future: F) -> F::Output
where
    F: Future,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(future)
}

pub fn upper(value: &Json) -> Json {
    match value {
        Json::Text(text) => Json::Text(text.to_uppercase()),
        other => other.clone(),
    }
}

fn dyn_mask(mask: &fn(&Json) -> Json) -> &dyn Fn(&Json) -> Json {
    mask
}

pub struct Inputs {
    pub metadata: IndexMap<String, Json>,
    pub litellm_params: Json,
    pub optional_params: IndexMap<String, Json>,
    pub payload: Option<StandardLoggingPayload>,
    pub response: Option<Json>,
    pub input: Option<Json>,
    pub output: Option<Json>,
    pub default_tags: Vec<String>,
    pub update_trace_keys: bool,
    pub redact: bool,
    pub environment: BTreeMap<String, String>,
    pub cache_hit: Option<bool>,
    pub preset_cache_key: Option<String>,
    pub call_type: &'static str,
    pub level: &'static str,
    pub model: &'static str,
    pub provider: Option<&'static str>,
    pub litellm_call_id: Option<&'static str>,
    pub litellm_trace_id: Option<&'static str>,
    pub start_time: Option<Timestamp>,
    pub end_time: Option<Timestamp>,
    pub response_cost: Option<Decimal>,
    pub vertex_location: Option<&'static str>,
    pub aws_region_name: Option<&'static str>,
    pub masker: Option<fn(&Json) -> Json>,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            metadata: IndexMap::new(),
            litellm_params: Json::Object(IndexMap::new()),
            optional_params: IndexMap::new(),
            payload: None,
            response: None,
            input: None,
            output: None,
            default_tags: Vec::new(),
            update_trace_keys: false,
            redact: false,
            environment: BTreeMap::new(),
            cache_hit: None,
            preset_cache_key: None,
            call_type: "completion",
            level: "DEFAULT",
            model: "gpt-4o",
            provider: None,
            litellm_call_id: None,
            litellm_trace_id: None,
            start_time: None,
            end_time: None,
            response_cost: None,
            vertex_location: None,
            aws_region_name: None,
            masker: None,
        }
    }
}

impl Inputs {
    pub fn trace(&self) -> GenerationTrace<'_> {
        GenerationTrace {
            call_type: self.call_type,
            level: self.level,
            model: self.model,
            provider: self.provider,
            metadata: &self.metadata,
            litellm_params: Some(&self.litellm_params),
            payload: self.payload.as_ref(),
            response: self.response.as_ref(),
            input: self.input.as_ref(),
            output: self.output.as_ref(),
            optional_params: self.optional_params.clone(),
            litellm_call_id: self.litellm_call_id,
            litellm_trace_id: self.litellm_trace_id,
            start_time: self.start_time,
            end_time: self.end_time,
            completion_start_time: None,
            response_cost: self.response_cost,
            vertex_location: self.vertex_location,
            aws_region_name: self.aws_region_name,
            cache_hit: self.cache_hit,
            preset_cache_key: self.preset_cache_key.as_deref(),
            masker: self.masker.as_ref().map(dyn_mask),
        }
    }

    pub fn settings(&self) -> AssemblySettings<'_> {
        AssemblySettings {
            default_tags: &self.default_tags,
            update_trace_keys: self.update_trace_keys,
            redact_user_api_key_info: self.redact,
        }
    }

    pub fn set_metadata(&mut self, name: &str, value: Json) {
        self.metadata.insert(name.to_owned(), value);
    }
}

pub struct Fixture {
    pub inputs: Inputs,
    provider: SdkTracerProvider,
    exporter: InMemorySpanExporter,
    client: LangfuseApiClient,
}

impl Default for Fixture {
    fn default() -> Self {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .with_id_generator(RequestedIdGenerator::default())
            .build();
        Self {
            inputs: Inputs::default(),
            provider,
            exporter,
            client: idle_client(),
        }
    }
}

impl Fixture {
    pub async fn assemble(&self) -> Outcome {
        let tracer = self.provider.tracer("langfuse-assembly");
        let generator = RequestedIdGenerator::default();
        let runtime = AssemblyRuntime {
            tracer: &tracer,
            id_generator: &generator,
            prompt_client: &self.client,
            environment: &self.inputs.environment,
        };
        let (trace_id, generation_id) =
            assemble_generation_trace(&self.inputs.trace(), &runtime, &self.inputs.settings())
                .await;
        Outcome {
            trace_id,
            generation_id,
            spans: self
                .exporter
                .get_finished_spans()
                .expect("the exported spans"),
        }
    }
}

pub struct Outcome {
    pub trace_id: Option<TraceId>,
    pub generation_id: Option<String>,
    pub spans: Vec<SpanData>,
}

impl Outcome {
    pub fn by_name(&self, name: &str) -> Option<&SpanData> {
        self.spans.iter().find(|span| span.name.to_string() == name)
    }

    pub fn names(&self) -> Vec<String> {
        self.spans
            .iter()
            .map(|span| span.name.to_string())
            .collect()
    }

    pub fn generation(&self) -> Option<&SpanData> {
        self.spans
            .iter()
            .find(|span| text(span, "langfuse.observation.type").as_deref() == Some("generation"))
    }
}

pub fn attribute(data: &SpanData, key: &str) -> Option<Value> {
    data.attributes
        .iter()
        .find(|pair| pair.key.as_str() == key)
        .map(|pair| pair.value.clone())
}

pub fn text(data: &SpanData, key: &str) -> Option<String> {
    match attribute(data, key)? {
        Value::String(text) => Some(String::from(text)),
        Value::Bool(_) | Value::I64(_) | Value::F64(_) | Value::Array(_) => None,
    }
}

pub fn json(data: &SpanData, key: &str) -> Option<Json> {
    serde_json::from_str(&text(data, key)?).ok()
}

pub fn object_field(value: &Json, name: &str) -> Option<Json> {
    match value {
        Json::Object(fields) => fields.get(name).cloned(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub fn absent(data: &SpanData, key: &str) -> bool {
    attribute(data, key).is_none()
}

pub fn seconds(value: i64) -> Timestamp {
    Timestamp::Seconds(Decimal::new(value, 0))
}

pub fn epoch_plus(seconds: u64) -> std::time::SystemTime {
    std::time::UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .expect("a timestamp")
}

struct WallClock;

impl Clock for WallClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

fn idle_client() -> LangfuseApiClient {
    let policy =
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("a proxy policy");
    let http = ProxiedClient::outbound(&Arc::new(policy)).expect("an HTTP client");
    build_langfuse_client(
        LangfuseClientConfig {
            public_key: Some("pk-lf-test".to_owned()),
            secret_key: Some(SecretString::from("sk-lf-test")),
            base_url: "http://127.0.0.1:1".to_owned(),
            timeout: Duration::from_secs(1),
            prompt_cache_ttl: Duration::ZERO,
        },
        Some(http),
        Arc::new(WallClock),
        TaskTracker::new(),
    )
    .expect("a Langfuse client")
}
