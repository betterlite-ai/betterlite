use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io::{Result as IoResult, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::langfuse_settings::Settings;
use betterlite_config::types::ProxyTrust;
use betterlite_env::EnvError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use opentelemetry::trace::{SpanId, TraceId};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::{IdGenerator, SpanData, SpanExporter};
use secrecy::SecretString;
use tokio::runtime::Handle;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::dynamic_logger_cache::{DynamicLoggerCache, RemovalEffect};
use crate::fields::payload::StandardLoggingPayload;
use crate::langfuse_channels::{LangfuseTracing, acquire_langfuse_tracing};
use crate::langfuse_logger::budget::LangfuseClientBudget;
use crate::langfuse_logger::langfuse::{LangFuseLogger, LangfuseCall};
use crate::langfuse_logger::{
    AssemblyDefaults, HttpClientBuilder, LangfuseClientBuilder, LangfuseLoggerRequest,
    LangfuseServices, channel_request,
};
use crate::langfuse_observations::identity::Timestamp;
use crate::langfuse_observations::requested_ids::RequestedIdGenerator;
use crate::langfuse_projections::content::ResponseObj;
use crate::langfuse_rest::cache::SystemClock;
use crate::langfuse_rest::client::{LangfuseApiClient, LangfuseClientConfig, LangfuseClientError};
use crate::langfuse_settings::{
    CredentialRequest, export, mock_mode, resolve_credentials, resolve_environment,
};

pub(crate) const HOST: &str = "http://127.0.0.1:1";
pub(crate) const TRACE_ID: &str = "0123456789abcdef0123456789abcdef";
// the ids FixedIds hands out, and the generation id the response and start time resolve to
pub(crate) const FIXED_TRACE_ID: &str = "11111111111111111111111111111111";
pub(crate) const GENERATION_ID: &str = "0b2c3384769faad7";

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "@trace_id",
    "session_id": "session-1",
    "call_type": "completion",
    "response_cost": 0.5,
    "status": "success",
    "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
    "total_tokens": 5,
    "prompt_tokens": 2,
    "completion_tokens": 3,
    "startTime": 1704067200.0,
    "endTime": 1704067201.0,
    "completionStartTime": 1704067200.5,
    "response_time": 1.0,
    "model_map_information": {},
    "model": "gpt-4o",
    "api_base": "https://api.openai.com",
    "metadata": {},
    "saved_cache_cost": 0.0,
    "request_tags": [],
    "model_parameters": {},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "the secret prompt"}],
    "response": null
}"#;

const RESPONSE: &str = r#"{
    "id": "chatcmpl-1",
    "object": "chat.completion",
    "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
    "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}
}"#;

pub(crate) fn lookup(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn settings(variables: &[(&str, &str)]) -> Settings {
    let lookup = lookup(variables);
    ConfigLoader { lookup }
        .load("")
        .expect("the langfuse settings load")
        .sections
        .langfuse_settings
}

pub(crate) fn proxied() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .expect("the proxy policy resolves");
    ProxiedClient::outbound(&Arc::new(policy)).expect("the client builds")
}

pub(crate) fn services(variables: &[(&str, &str)], limit: usize) -> LangfuseServices {
    services_with(
        variables,
        LangfuseClientBudget::new(limit),
        real_builder(),
        Arc::new(RequestedIdGenerator::default()),
    )
}

pub(crate) fn services_with(
    variables: &[(&str, &str)],
    budget: LangfuseClientBudget,
    clients: Arc<dyn LangfuseClientBuilder>,
    id_generator: Arc<dyn IdGenerator>,
) -> LangfuseServices {
    let environment = lookup(variables);
    LangfuseServices {
        settings: settings(variables),
        environment: Arc::new(environment),
        http: proxied(),
        budget,
        tasks: TaskTracker::new(),
        runtime: Handle::current(),
        assembly: AssemblyDefaults::default(),
        id_generator,
        clients,
    }
}

pub(crate) fn real_builder() -> Arc<dyn LangfuseClientBuilder> {
    Arc::new(HttpClientBuilder {
        http: proxied(),
        clock: Arc::new(SystemClock),
        tasks: TaskTracker::new(),
    })
}

pub(crate) struct FailingBuilder;

impl LangfuseClientBuilder for FailingBuilder {
    fn build(
        &self,
        _config: LangfuseClientConfig,
    ) -> Result<LangfuseApiClient, LangfuseClientError> {
        Err(LangfuseClientError::Proxy {
            source: EnvError::Number {
                name: "HTTPS_PROXY".to_owned(),
            },
        })
    }
}

pub(crate) fn request<'request>(
    public_key: &'request str,
    secret: &'request str,
) -> LangfuseLoggerRequest<'request> {
    LangfuseLoggerRequest {
        credentials: CredentialRequest {
            public_key: Some(public_key),
            secret: Some(secret),
            secret_key: None,
            host: Some(HOST),
            allow_env_credentials: false,
        },
        environment: None,
        // the seeded channel never exports on its schedule, so only an explicit flush delivers spans
        flush_interval: u64::MAX,
    }
}

pub(crate) fn environment_request() -> LangfuseLoggerRequest<'static> {
    LangfuseLoggerRequest {
        credentials: CredentialRequest {
            public_key: None,
            secret: None,
            secret_key: None,
            host: None,
            allow_env_credentials: true,
        },
        environment: None,
        flush_interval: 1,
    }
}

// put the export channel the logger will look up under its own key, backed by a recorder
pub(crate) fn seed<E>(
    services: &LangfuseServices,
    request: &LangfuseLoggerRequest<'_>,
    exporter: E,
) -> LangfuseTracing
where
    E: SpanExporter + 'static,
{
    let credentials = resolve_credentials(&services.settings, request.credentials);
    let environment = resolve_environment(&services.settings, request.environment)
        .expect("the environment resolves");
    let mock = mock_mode(&services.settings).expect("the mock flag parses");
    let flush_interval = export::flush_interval(&services.settings, request.flush_interval);
    let channel = channel_request(
        &services.settings,
        &credentials,
        environment,
        flush_interval,
        mock,
    );
    acquire_langfuse_tracing(
        &services.settings,
        services.environment.as_ref(),
        &services.runtime,
        &channel,
        || Ok(exporter),
    )
    .expect("the channel is acquired")
}

pub(crate) fn payload(trace_id: &str) -> StandardLoggingPayload {
    let source = PAYLOAD.replace("@trace_id", trace_id);
    serde_json::from_str(&source).expect("the payload fixture parses")
}

pub(crate) fn response() -> Json {
    serde_json::from_str(RESPONSE).expect("the response fixture parses")
}

pub(crate) fn failed_response() -> Json {
    serde_json::from_str(r#"{"id":"chatcmpl-1","usage":{"prompt_tokens":"many"}}"#)
        .expect("the failing response fixture parses")
}

pub(crate) fn optional_params() -> IndexMap<String, Json> {
    let mut params = IndexMap::new();
    params.insert(
        "temperature".to_owned(),
        serde_json::from_str("0.2").expect("a number"),
    );
    params.insert(
        "tools".to_owned(),
        serde_json::from_str(r#"[{"type":"function","function":{"name":"lookup"}}]"#)
            .expect("the tools fixture parses"),
    );
    params.insert("functions".to_owned(), Json::Null);
    params.insert(
        "secret_fields".to_owned(),
        Json::List(vec![Json::Text("authorization".to_owned())]),
    );
    params.insert(
        "custom".to_owned(),
        serde_json::from_str(r#"{"nested":1}"#).expect("the custom fixture parses"),
    );
    params
}

pub(crate) fn litellm_params(debug: bool) -> Json {
    let source = if debug {
        r#"{
            "metadata": {"user_api_key_alias": "alias-1", "debug_langfuse": "true"},
            "proxy_server_request": {"headers": {"langfuse_trace_name": "from-header"}}
        }"#
    } else {
        r#"{
            "metadata": {"user_api_key_alias": "alias-1"},
            "proxy_server_request": {"headers": {"langfuse_trace_name": "from-header"}}
        }"#
    };
    serde_json::from_str(source).expect("the litellm_params fixture parses")
}

pub(crate) fn start() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(1_704_067_200, 0).expect("the start time")
}

pub(crate) fn call<'call>(
    payload: &'call StandardLoggingPayload,
    response: &'call Json,
    optional_params: &'call IndexMap<String, Json>,
    litellm_params: &'call Json,
    start: DateTime<Utc>,
) -> LangfuseCall<'call> {
    LangfuseCall {
        payload,
        litellm_params: Some(litellm_params),
        messages: payload.messages.as_ref(),
        optional_params,
        response: Some(response),
        response_object: Some(ResponseObj::Chat(response)),
        start_time: Some(Timestamp::DateTime(start)),
        end_time: None,
        completion_start_time: None,
        level: "DEFAULT",
        status_message: None,
        litellm_trace_id: None,
        vertex_location: None,
        aws_region_name: None,
        preset_cache_key: None,
        input: None,
        masker: None,
    }
}

#[derive(Clone, Default)]
pub(crate) struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    pub(crate) fn contents(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("the log is utf-8")
    }
}

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Buffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture_log() -> (Buffer, DefaultGuard) {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Recording {
    state: Arc<RecordingState>,
}

#[derive(Debug, Default)]
struct RecordingState {
    spans: Mutex<Vec<SpanData>>,
    installed: AtomicUsize,
    shut_down: AtomicUsize,
}

impl Recording {
    pub(crate) fn spans(&self) -> Vec<SpanData> {
        self.state
            .spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn installed(&self) -> usize {
        self.state.installed.load(Ordering::Acquire)
    }

    pub(crate) fn shut_down(&self) -> usize {
        self.state.shut_down.load(Ordering::Acquire)
    }
}

impl SpanExporter for Recording {
    fn export(&self, batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
        self.state
            .spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(batch);
        ready(Ok(()))
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        self.state.shut_down.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn set_resource(&mut self, _resource: &Resource) {
        self.state.installed.fetch_add(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FixedIds;

impl IdGenerator for FixedIds {
    fn new_trace_id(&self) -> TraceId {
        TraceId::from_bytes([0x11; 16])
    }

    fn new_span_id(&self) -> SpanId {
        SpanId::from_bytes([0x22; 8])
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Rejecting;

impl SpanExporter for Rejecting {
    fn export(&self, _batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
        ready(Err(OTelSdkError::InternalFailure(
            "the collector refused the batch".to_owned(),
        )))
    }
}

pub(crate) fn recording() -> (Recording, Recording) {
    let recorder = Recording::default();
    (recorder.clone(), recorder)
}

pub(crate) fn attribute_text(spans: &[SpanData], key: &str) -> Option<String> {
    spans.iter().find_map(|span| {
        span.attributes
            .iter()
            .find(|attribute| attribute.key.as_str() == key)
            .map(|attribute| attribute.value.as_str().into_owned())
    })
}

pub(crate) fn attribute_json(spans: &[SpanData], key: &str) -> Json {
    let text = attribute_text(spans, key).expect("the span carries the attribute");
    serde_json::from_str(&text).expect("the attribute holds json")
}

pub(crate) fn object(value: Json) -> IndexMap<String, Json> {
    match value {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the attribute is an object")
        }
    }
}

pub(crate) fn cache() -> DynamicLoggerCache<Arc<LangFuseLogger>> {
    let removal: RemovalEffect<Arc<LangFuseLogger>> = Arc::new(|logger| logger.stop());
    DynamicLoggerCache::new(removal, Arc::new(|| 0.0_f64))
}

pub(crate) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}
