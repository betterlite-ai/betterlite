use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::service_trace_id_cache::ServiceTraceIDCache;
use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::task::TaskTracker;

use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::langfuse_channels::LangfuseTracing;
use crate::langfuse_logger::LangfuseClientBuilder;
use crate::langfuse_logger::budget::LangfuseClientBudget;
use crate::langfuse_prompt_management::LangfusePromptManagement;
use crate::langfuse_rest::cache::SystemClock;
use crate::langfuse_rest::client::{
    LangfuseApiClient, LangfuseClientConfig, LangfuseClientError, build_langfuse_client,
};

pub(super) use crate::langfuse_logger::tests::support as langfuse;

pub(super) const HOST: &str = "http://127.0.0.1:1";
pub(super) const TRACE_ID: &str = crate::langfuse_logger::tests::support::TRACE_ID;
pub(super) const PUBLIC_KEY: &str = "pk-lf-test";
pub(super) const SECRET_KEY: &str = "sk-lf-test";
pub(super) const CALL_ID: &str = "call-1";
pub(super) const PROJECTS: &str = r#"{"data":[{"id":"project-one","name":"One","organization":{"id":"org-1","name":"Org"},"metadata":{}}]}"#;

const PAYLOAD: &str = r#"{
    "id": "chatcmpl-1",
    "trace_id": "@trace_id",
    "litellm_call_id": @call_id,
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
    "model_parameters": {"temperature": 0.2},
    "hidden_params": {},
    "messages": [{"role": "user", "content": "the secret prompt"}],
    "response": @response,
    "error_str": @error
}"#;

const RESPONSE: &str = r#"{
    "id": "chatcmpl-1",
    "object": "chat.completion",
    "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
    "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}
}"#;

const CHAT_PROMPT: &str = r#"{"type":"chat","prompt":[{"role":"system","content":"Hello {{name}}"},{"name":"history"}],"name":"greeting","version":3,"config":{"model":"gpt-4o","temperature":0.2},"labels":["production"],"tags":[]}"#;

const TEXT_PROMPT: &str = r#"{"type":"text","prompt":"You are a {{role}}.","name":"intro","version":1,"config":{"model":"claude-3-haiku"},"labels":[],"tags":[]}"#;

pub(super) struct Fixture {
    pub(super) manager: LangfusePromptManagement,
    pub(super) spans: langfuse::Recording,
    _probe: LangfuseTracing,
}

pub(super) fn env(host: &str) -> Vec<(&'static str, String)> {
    vec![
        ("LANGFUSE_HOST", host.to_owned()),
        ("LANGFUSE_PUBLIC_KEY", PUBLIC_KEY.to_owned()),
        ("LANGFUSE_SECRET_KEY", SECRET_KEY.to_owned()),
    ]
}

pub(super) fn borrowed(variables: &[(&'static str, String)]) -> Vec<(&'static str, &str)> {
    variables
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect()
}

pub(super) fn services(
    variables: &[(&str, &str)],
    clients: Arc<dyn LangfuseClientBuilder>,
) -> crate::langfuse_logger::LangfuseServices {
    langfuse::services_with(
        variables,
        LangfuseClientBudget::new(100),
        clients,
        Arc::new(langfuse::FixedIds),
    )
}

pub(super) async fn manager(variables: &[(&str, &str)]) -> Fixture {
    let (exporter, spans) = langfuse::recording();
    let services = services(variables, langfuse::real_builder());
    let probe = langfuse::seed(&services, &langfuse::environment_request(), exporter);
    let trace_cache = trace_cache();
    let manager = LangfusePromptManagement::new(services, trace_cache)
        .await
        .expect("the manager builds");
    Fixture {
        manager,
        spans,
        _probe: probe,
    }
}

pub(super) fn trace_cache() -> Arc<ServiceTraceIDCache> {
    Arc::new(ServiceTraceIDCache::new(Arc::new(|| 0.0_f64)).expect("the trace cache builds"))
}

pub(super) fn response() -> Json {
    serde_json::from_str(RESPONSE).expect("the response parses")
}

pub(super) fn chat_prompt() -> String {
    CHAT_PROMPT.to_owned()
}

pub(super) fn text_prompt() -> String {
    TEXT_PROMPT.to_owned()
}

pub(super) fn payload(
    call_id: Option<&str>,
    response: Json,
    error: Option<&str>,
) -> StandardLoggingPayload {
    let call_id = serde_json::to_string(&call_id).expect("the call id serializes");
    let response = serde_json::to_string(&response).expect("the response serializes");
    let error = serde_json::to_string(&error).expect("the error serializes");
    let source = PAYLOAD
        .replace("@trace_id", langfuse::TRACE_ID)
        .replace("@call_id", &call_id)
        .replace("@response", &response)
        .replace("@error", &error);
    serde_json::from_str(&source).expect("the payload parses")
}

pub(super) fn success_payload(call_id: Option<&str>) -> StandardLoggingPayload {
    payload(call_id, response(), None)
}

pub(super) fn failure_payload(
    call_id: Option<&str>,
    error: Option<&str>,
) -> StandardLoggingPayload {
    payload(call_id, Json::Null, error)
}

pub(super) fn event(payload: StandardLoggingPayload) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    })
}

pub(super) fn no_dynamic_params() -> StandardCallbackDynamicParams {
    Json::Object(IndexMap::new())
}

pub(super) fn dynamic_params(entries: &[(&str, &str)]) -> StandardCallbackDynamicParams {
    Json::Object(
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), Json::Text((*value).to_owned())))
            .collect(),
    )
}

// a scripted Langfuse REST server: one scripted answer per request, in order, then empty objects
pub(super) struct FakeLangfuse {
    pub(super) base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeLangfuse {
    pub(super) async fn start(answers: Vec<(u16, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let scripted = Arc::new(Mutex::new(VecDeque::from(answers)));
        let seen = Arc::clone(&requests);
        tokio::spawn(async move {
            while let Ok((stream, _peer)) = listener.accept().await {
                let seen = Arc::clone(&seen);
                let scripted = Arc::clone(&scripted);
                tokio::spawn(async move { answer(stream, seen, scripted).await });
            }
        });
        Self {
            base_url: format!("http://{address}"),
            requests,
        }
    }

    pub(super) fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

async fn answer(
    mut stream: TcpStream,
    requests: Arc<Mutex<Vec<String>>>,
    answers: Arc<Mutex<VecDeque<(u16, String)>>>,
) {
    let mut buffer = vec![0_u8; 8192];
    let Ok(read) = stream.read(&mut buffer).await else {
        return;
    };
    let head = String::from_utf8_lossy(&buffer[..read]).into_owned();
    requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(head);
    let (status, body) = {
        let mut answers = answers.lock().unwrap_or_else(PoisonError::into_inner);
        answers.pop_front().unwrap_or((200, "{}".to_owned()))
    };
    let response = format!(
        "HTTP/1.1 {status} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        reason(status),
        body.len()
    );
    let _written = stream.write_all(response.as_bytes()).await;
}

const fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Response",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedConfig {
    pub(super) public_key: Option<String>,
    pub(super) secret: Option<String>,
    pub(super) base_url: String,
}

// records the client config langfuse_client_init would build, then builds the real client
#[derive(Clone, Default)]
pub(super) struct RecordingBuilder {
    configs: Arc<Mutex<Vec<RecordedConfig>>>,
}

impl RecordingBuilder {
    pub(super) fn configs(&self) -> Vec<RecordedConfig> {
        self.configs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl LangfuseClientBuilder for RecordingBuilder {
    fn build(
        &self,
        config: LangfuseClientConfig,
    ) -> Result<LangfuseApiClient, LangfuseClientError> {
        let recorded = RecordedConfig {
            public_key: config.public_key.clone(),
            secret: config
                .secret_key
                .as_ref()
                .map(|secret| secret.expose_secret().to_owned()),
            base_url: config.base_url.clone(),
        };
        self.configs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        build_langfuse_client(
            config,
            Some(langfuse::proxied()),
            Arc::new(SystemClock),
            TaskTracker::new(),
        )
    }
}
