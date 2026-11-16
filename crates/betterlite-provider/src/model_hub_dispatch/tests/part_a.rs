use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::model_hub_credentials::Settings;
use betterlite_context::chat_config_selection::tags::{
    ChatConfigTag, ConfigConstruction, ConfigSelection,
};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{StreamResponse, UpstreamResponse};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::huggingface_chat::HuggingfaceEnv;
use betterlite_openai::huggingface_chat::mapping::HubResponse;
use betterlite_openai::huggingface_embedding::error::HuggingfaceEmbeddingError;
use betterlite_openai::huggingface_embedding::response::{EmbeddingResponse, OrdinaryEncoder};
use betterlite_openai::huggingface_embedding::task::PipelineTagSource;
use betterlite_openai::huggingface_rerank::TextTokenCounter;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::count::error::CountError;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::credentials::{
    Globals, huggingface_chat_key, huggingface_embedding_key, huggingface_env,
    huggingface_hub_token, huggingface_rerank_base, huggingface_rerank_key, predibase_base,
    predibase_key, predibase_tenant_id, replicate_base, replicate_key,
};
use super::embedding::{EmbeddingCall, embed};
use super::hub::{HubGetAsync, PrefetchedTag};
use super::rerank::huggingface_rerank_config;
use super::tokens::Cl100k;
use super::transport::PostTransport;
use super::{
    CallLogging, ChatContext, DispatchDeps, HubCompletion, HubDispatchError, HubProvider,
    ReplicateServices, dispatch, polling, predibase, select,
};
use crate::api::rerank_contracts::{BaseRerankConfig, EnvironmentRequest};
use crate::oci_common::Clock;
use crate::operation::OperationError;
use crate::predibase::error::PredibaseSetupError;
use crate::predibase::reply::TokenCounter as PredibaseTokenCounter;
use crate::prepared::PreparedRequest;
use crate::replicate::poll::transport::{ReplicateRequest, ReplicateResponse, ReplicateTransport};
use crate::replicate::poll::{ReplicateCompletion, ReplicatePolling, ReplicateSleeper};
use crate::replicate::reply::{PostCall, PostCallLogger, TokenCounter as ReplicateTokenCounter};

const INIT: &str = r#"{"model":"gpt-4o","call_type":"embedding","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW_SECONDS: i64 = 1_700_000_000;
const CREATE: &str = r#"{"urls":{"get":"https://poll/1"}}"#;
const DONE: &str = r#"{"status":"succeeded","output":["ok"]}"#;
const MODEL: &str = "meta-llama/llama-2-70b-chat";
const HF_MODEL: &str = "meta-llama/Llama-3-8B-Instruct";
const PB_MODEL: &str = "meta-llama/llama-2-7b-chat";
const PROMPT: &str = r#"{"input":{"prompt":"<s>[INST] hi [/INST]\n"}}"#;

struct Discard;

impl LogSink for Discard {
    fn emit(&self, _record: LogRecord) {}
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn text(value: Option<&SecretString>) -> Option<String> {
    value.map(|secret| secret.expose_secret().to_owned())
}

fn empty_globals<'source>() -> Globals<'source> {
    Globals {
        api_key: None,
        huggingface_key: None,
        replicate_key: None,
        predibase_key: None,
        predibase_tenant_id: None,
        api_base: None,
        headers: None,
        custom_prompt_dict: None,
    }
}

fn object(pairs: &[(&str, PromptJson)]) -> Object {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn number(value: i64) -> PromptJson {
    PromptJson::Number(serde_json::Number::from(value))
}

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(NOW_SECONDS, 0).unwrap()
}

fn user_message() -> Message {
    serde_json::from_str(r#"{"role":"user","content":"hi"}"#).unwrap()
}

fn hub_reply(status: u16, body: &str) -> HubResponse {
    HubResponse {
        status_code: status,
        headers: Headers::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn http_reply(status: u16, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

struct ScriptedHub {
    replies: Mutex<VecDeque<HubResponse>>,
    requests: Mutex<Vec<(String, Headers, Vec<(String, String)>)>>,
}

impl ScriptedHub {
    fn new(replies: Vec<HubResponse>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<(String, Headers, Vec<(String, String)>)> {
        self.requests.lock().unwrap().clone()
    }

    fn calls(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[async_trait]
impl HubGetAsync for ScriptedHub {
    async fn get(
        &self,
        url: &str,
        headers: &Headers,
        query: &[(&str, &str)],
    ) -> Result<HubResponse, HubDispatchError> {
        let recorded = query
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        self.requests
            .lock()
            .unwrap()
            .push((url.to_owned(), headers.clone(), recorded));
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(HubDispatchError::BodyNotObject)
    }
}

struct ScriptedPost {
    replies: Mutex<VecDeque<UpstreamResponse>>,
    requests: Mutex<Vec<(String, Headers, String)>>,
    stream_requests: Mutex<Vec<(String, Headers, String)>>,
}

impl ScriptedPost {
    fn new(replies: Vec<UpstreamResponse>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
            stream_requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<(String, Headers, String)> {
        self.requests.lock().unwrap().clone()
    }

    fn stream_requests(&self) -> Vec<(String, Headers, String)> {
        self.stream_requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl PostTransport for ScriptedPost {
    async fn post(
        &self,
        url: &str,
        headers: &Headers,
        body: &[u8],
        _deadline: Duration,
    ) -> Result<UpstreamResponse, HubDispatchError> {
        let sent = String::from_utf8(body.to_vec()).unwrap();
        self.requests
            .lock()
            .unwrap()
            .push((url.to_owned(), headers.clone(), sent));
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(HubDispatchError::BodyNotObject)
    }

    async fn post_stream(
        &self,
        url: &str,
        headers: &Headers,
        body: &[u8],
        _deadline: Duration,
    ) -> Result<StreamResponse, HubDispatchError> {
        let sent = String::from_utf8(body.to_vec()).unwrap();
        self.stream_requests
            .lock()
            .unwrap()
            .push((url.to_owned(), headers.clone(), sent));
        Err(HubDispatchError::Operation(
            OperationError::UnsupportedOperation {
                provider: String::from("predibase"),
                operation: String::from("stream"),
            },
        ))
    }
}

struct ScriptedTransport {
    replies: Mutex<VecDeque<ReplicateResponse>>,
    requests: Mutex<Vec<ReplicateRequest>>,
}

impl ScriptedTransport {
    fn new(replies: Vec<ReplicateResponse>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<ReplicateRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl ReplicateTransport for ScriptedTransport {
    async fn send(&self, request: ReplicateRequest) -> Result<ReplicateResponse, TransportError> {
        self.requests.lock().unwrap().push(request);
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(TransportError::Closed)
    }
}

fn replicate_reply(status: u16, body: &str) -> ReplicateResponse {
    ReplicateResponse {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

struct RecordingSleeper {
    delays: Mutex<Vec<f64>>,
}

impl RecordingSleeper {
    fn new() -> Self {
        Self {
            delays: Mutex::new(Vec::new()),
        }
    }

    fn delays(&self) -> Vec<f64> {
        self.delays.lock().unwrap().clone()
    }
}

#[async_trait]
impl ReplicateSleeper for RecordingSleeper {
    async fn sleep(&self, seconds: f64) {
        self.delays.lock().unwrap().push(seconds);
    }
}

struct SilentLogger;

impl PostCallLogger for SilentLogger {
    fn post_call(&self, _call: PostCall<'_>) {}
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        now()
    }
}

struct LengthEncoder;

impl OrdinaryEncoder for LengthEncoder {
    fn encode_ordinary(&self, text: &str) -> usize {
        text.chars().count()
    }
}

struct Counting;

#[async_trait]
impl ReplicateTokenCounter for Counting {
    async fn count_messages(&self, _model: &str, messages: &[Message]) -> Result<i64, CountError> {
        i64::try_from(messages.len()).map_err(|_overflow| CountError::Overflow)
    }

    async fn count_response_tokens(&self, _model: &str, text: &str) -> Result<i64, CountError> {
        i64::try_from(text.chars().count()).map_err(|_overflow| CountError::Overflow)
    }
}

struct PlainCounting;

#[async_trait]
impl PredibaseTokenCounter for PlainCounting {
    async fn count_messages(&self, messages: &[Message]) -> Option<i64> {
        i64::try_from(messages.len()).ok()
    }

    async fn count_text(&self, text: &str) -> Option<i64> {
        i64::try_from(text.chars().count()).ok()
    }
}

struct Services {
    hub: ScriptedHub,
    post: ScriptedPost,
    transport: Arc<ScriptedTransport>,
    sleeper: Arc<RecordingSleeper>,
    logger: SilentLogger,
    prompts: PromptFactory,
    replicate_counter: Counting,
    predibase_counter: PlainCounting,
    encoder: LengthEncoder,
    clock: FixedClock,
}

impl Services {
    fn new(hub: ScriptedHub, post: ScriptedPost, transport: ScriptedTransport) -> Self {
        Self {
            hub,
            post,
            transport: Arc::new(transport),
            sleeper: Arc::new(RecordingSleeper::new()),
            logger: SilentLogger,
            prompts: PromptFactory {
                cache: Mutex::new(builtin_entries()),
                hub: Hub::Unreachable,
            },
            replicate_counter: Counting,
            predibase_counter: PlainCounting,
            encoder: LengthEncoder,
            clock: FixedClock,
        }
    }

    fn idle() -> Self {
        Self::new(
            ScriptedHub::new(Vec::new()),
            ScriptedPost::new(Vec::new()),
            ScriptedTransport::new(Vec::new()),
        )
    }

    fn deps<'deps>(
        &'deps self,
        globals: &'deps Globals<'deps>,
        environment: &'deps BTreeMap<String, String>,
    ) -> DispatchDeps<'deps> {
        DispatchDeps {
            globals,
            environment,
            hub: &self.hub,
            post: &self.post,
            encoder: &self.encoder,
            replicate: ReplicateServices {
                transport: Arc::clone(&self.transport),
                sleeper: Arc::clone(&self.sleeper),
                logger: &self.logger,
            },
            prompts: &self.prompts,
            replicate_counter: &self.replicate_counter,
            predibase_counter: &self.predibase_counter,
            clock: &self.clock,
            default_max_tokens: 4096,
            now: now(),
            timeout: Duration::from_secs(5),
        }
    }
}

fn logging(redactor: &Redactor) -> (LoggingState, PrePostEnv<'_>) {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "trace".to_owned(),
        Duration::default,
    )
    .unwrap();
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Discard,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap(),
    };
    (state, env)
}

fn chat_context<'source>(
    model: &'source str,
    messages: &'source mut Vec<Message>,
    options: &'source mut Object,
    api_base: Option<&'source str>,
    api_key: Option<&'source SecretString>,
) -> ChatContext<'source> {
    ChatContext {
        model,
        messages,
        optional_params: options,
        api_key,
        api_base,
        headers: None,
        custom_prompt_dict: None,
        acompletion: false,
    }
}

fn selection(tag: ChatConfigTag) -> ConfigSelection<ChatConfigTag> {
    ConfigSelection {
        tag,
        construction: ConfigConstruction::Shared,
    }
}

fn embedding_call<'call>(
    input: &'call [String],
    options: &'call Object,
    headers: &'call Headers,
    api_base: Option<&'call str>,
) -> EmbeddingCall<'call> {
    EmbeddingCall {
        model: "org/model",
        input,
        optional_params: options,
        api_key: None,
        api_base,
        headers,
    }
}

async fn embed_with(
    services: &Services,
    env: &BTreeMap<String, String>,
    call: &EmbeddingCall<'_>,
) -> (Result<EmbeddingResponse, HubDispatchError>, LoggingState) {
    let globals = empty_globals();
    let deps = services.deps(&globals, env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let outcome = embed(call, &deps, &mut log).await;
    (outcome, state)
}

async fn dispatch_with(
    services: &Services,
    env: &BTreeMap<String, String>,
    tag: ChatConfigTag,
    call: &mut ChatContext<'_>,
) -> (
    Result<Option<HubCompletion>, HubDispatchError>,
    LoggingState,
) {
    let globals = empty_globals();
    let deps = services.deps(&globals, env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let outcome = dispatch(&selection(tag), call, &deps, &mut log).await;
    (outcome, state)
}

#[test]
fn select_maps_only_the_three_hub_tags() {
    assert_eq!(
        select(ChatConfigTag::HuggingFaceChat),
        Some(HubProvider::HuggingFace)
    );
    assert_eq!(
        select(ChatConfigTag::Replicate),
        Some(HubProvider::Replicate)
    );
    assert_eq!(
        select(ChatConfigTag::Predibase),
        Some(HubProvider::Predibase)
    );
    assert_eq!(select(ChatConfigTag::OpenAiGpt), None);
    assert_eq!(select(ChatConfigTag::Anthropic), None);
}

#[tokio::test]
async fn dispatch_leaves_an_unowned_tag_alone() {
    let services = Services::idle();
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context("m", &mut messages, &mut options, None, None);
    let (outcome, state) = dispatch_with(
        &services,
        &environment(&[]),
        ChatConfigTag::OpenAiGpt,
        &mut call,
    )
    .await;
    assert!(outcome.unwrap().is_none());
    assert!(services.post.requests().is_empty());
    assert!(services.transport.requests().is_empty());
    assert!(state.model_call_details.get("log_event_type").is_none());
}

#[test]
fn the_settings_become_the_module_globals() {
    let settings = Settings {
        huggingface_key: Some(secret("hf")),
        replicate_key: Some(secret("r")),
        predibase_key: Some(secret("p")),
        predibase_tenant_id: Some(secret("t")),
    };
    let globals = Globals::from_settings(&settings);
    assert_eq!(text(globals.huggingface_key), Some(String::from("hf")));
    assert_eq!(text(globals.replicate_key), Some(String::from("r")));
    assert_eq!(text(globals.predibase_key), Some(String::from("p")));
    assert_eq!(text(globals.predibase_tenant_id), Some(String::from("t")));
    assert!(globals.api_key.is_none());
    assert!(globals.api_base.is_none());
}

#[test]
fn huggingface_chat_key_follows_litellm_order() {
    let context = secret("ctx");
    let hf_global = secret("hf-global");
    let api = secret("api-global");
    let mut globals = empty_globals();
    globals.huggingface_key = Some(&hf_global);
    globals.api_key = Some(&api);
    let both = environment(&[("HF_TOKEN", "hf-token"), ("HUGGINGFACE_API_KEY", "hf-api")]);
    assert_eq!(
        text(huggingface_chat_key(Some(&context), &globals, &both).as_ref()),
        Some(String::from("ctx"))
    );
    assert_eq!(
        text(huggingface_chat_key(None, &globals, &both).as_ref()),
        Some(String::from("hf-global"))
    );
    globals.huggingface_key = None;
    assert_eq!(
        text(huggingface_chat_key(None, &globals, &both).as_ref()),
        Some(String::from("hf-token"))
    );
    let only_api = environment(&[("HUGGINGFACE_API_KEY", "hf-api")]);
    assert_eq!(
        text(huggingface_chat_key(None, &globals, &only_api).as_ref()),
        Some(String::from("hf-api"))
    );
    let empty_token = environment(&[("HF_TOKEN", ""), ("HUGGINGFACE_API_KEY", "hf-api")]);
    assert_eq!(
        text(huggingface_chat_key(None, &globals, &empty_token).as_ref()),
        Some(String::from("hf-api"))
    );
    assert_eq!(
        text(huggingface_chat_key(None, &globals, &environment(&[])).as_ref()),
        Some(String::from("api-global"))
    );
    globals.api_key = None;
    assert!(huggingface_chat_key(None, &globals, &environment(&[])).is_none());
}

#[test]
fn huggingface_embedding_and_rerank_keys_skip_hf_token() {
    let hf_global = secret("hf-global");
    let api = secret("api-global");
    let mut globals = empty_globals();
    globals.huggingface_key = Some(&hf_global);
    globals.api_key = Some(&api);
    let both = environment(&[("HF_TOKEN", "hf-token"), ("HUGGINGFACE_API_KEY", "hf-api")]);
    assert_eq!(
        text(huggingface_embedding_key(None, &globals, &both).as_ref()),
        Some(String::from("hf-global"))
    );
    globals.huggingface_key = None;
    assert_eq!(
        text(huggingface_embedding_key(None, &globals, &both).as_ref()),
        Some(String::from("hf-api"))
    );
    assert_eq!(
        text(huggingface_embedding_key(None, &globals, &environment(&[])).as_ref()),
        Some(String::from("api-global"))
    );
    assert_eq!(
        text(huggingface_rerank_key(None, &globals, &both).as_ref()),
        Some(String::from("hf-api"))
    );
    assert!(huggingface_rerank_key(None, &globals, &environment(&[])).is_none());
    assert_eq!(
        text(huggingface_hub_token(&both).as_ref()),
        Some(String::from("hf-api"))
    );
    assert!(huggingface_hub_token(&environment(&[("HUGGINGFACE_API_KEY", "")])).is_none());
}

#[test]
fn huggingface_env_keeps_a_variable_set_to_the_empty_string() {
    let present = environment(&[
        ("HF_API_BASE", ""),
        ("HUGGINGFACE_API_BASE", "http://huggingface"),
    ]);
    assert_eq!(
        huggingface_env(&present),
        HuggingfaceEnv {
            hf_api_base: Some(String::new()),
            huggingface_api_base: Some(String::from("http://huggingface")),
        }
    );
    assert_eq!(
        huggingface_env(&environment(&[])),
        HuggingfaceEnv {
            hf_api_base: None,
            huggingface_api_base: None,
        }
    );
}

#[test]
fn huggingface_rerank_base_reads_the_call_then_the_two_variables() {
    let both = environment(&[
        ("HF_API_BASE", "http://hf"),
        ("HUGGINGFACE_API_BASE", "http://huggingface"),
    ]);
    assert_eq!(
        huggingface_rerank_base(Some("http://call"), &both).as_deref(),
        Some("http://call")
    );
    assert_eq!(
        huggingface_rerank_base(None, &both).as_deref(),
        Some("http://hf")
    );
    let empty_first = environment(&[
        ("HF_API_BASE", ""),
        ("HUGGINGFACE_API_BASE", "http://huggingface"),
    ]);
    assert_eq!(
        huggingface_rerank_base(None, &empty_first).as_deref(),
        Some("")
    );
    let only_second = environment(&[("HUGGINGFACE_API_BASE", "http://huggingface")]);
    assert_eq!(
        huggingface_rerank_base(None, &only_second).as_deref(),
        Some("http://huggingface")
    );
    assert!(huggingface_rerank_base(None, &environment(&[])).is_none());
}

#[test]
fn replicate_key_follows_litellm_order() {
    let context = secret("ctx");
    let replicate_global = secret("replicate-global");
    let api = secret("api-global");
    let mut globals = empty_globals();
    globals.replicate_key = Some(&replicate_global);
    globals.api_key = Some(&api);
    let both = environment(&[
        ("REPLICATE_API_KEY", "replicate-env"),
        ("REPLICATE_API_TOKEN", "replicate-token"),
    ]);
    assert_eq!(
        text(replicate_key(Some(&context), &globals, &both).as_ref()),
        Some(String::from("ctx"))
    );
    assert_eq!(
        text(replicate_key(None, &globals, &both).as_ref()),
        Some(String::from("replicate-global"))
    );
    globals.replicate_key = None;
    assert_eq!(
        text(replicate_key(None, &globals, &both).as_ref()),
        Some(String::from("api-global"))
    );
    globals.api_key = None;
    assert_eq!(
        text(replicate_key(None, &globals, &both).as_ref()),
        Some(String::from("replicate-env"))
    );
    let token_only = environment(&[("REPLICATE_API_TOKEN", "replicate-token")]);
    assert_eq!(
        text(replicate_key(None, &globals, &token_only).as_ref()),
        Some(String::from("replicate-token"))
    );
    assert!(replicate_key(None, &globals, &environment(&[])).is_none());
}

#[test]
fn replicate_base_follows_litellm_order() {
    let mut globals = empty_globals();
    let both = environment(&[("REPLICATE_API_BASE", "http://replicate")]);
    assert_eq!(
        replicate_base(Some("http://call"), &globals, &both),
        "http://call"
    );
    globals.api_base = Some("http://global");
    assert_eq!(replicate_base(None, &globals, &both), "http://global");
    globals.api_base = None;
    assert_eq!(replicate_base(None, &globals, &both), "http://replicate");
    assert_eq!(
        replicate_base(None, &globals, &environment(&[])),
        "https://api.replicate.com/v1"
    );
    let empty = environment(&[("REPLICATE_API_BASE", "")]);
    assert_eq!(
        replicate_base(None, &globals, &empty),
        "https://api.replicate.com/v1"
    );
}

#[test]
fn predibase_pops_tenant_id_before_predibase_tenant_id() {
    let tenant = secret("tenant-global");
    let mut globals = empty_globals();
    globals.predibase_tenant_id = Some(&tenant);
    let env = environment(&[("PREDIBASE_TENANT_ID", "tenant-env")]);
    let param = PromptJson::Text(String::from("tenant-param"));
    let mut both = object(&[
        ("tenant_id", PromptJson::Text(String::from("tenant-call"))),
        ("predibase_tenant_id", param.clone()),
    ]);
    let first = predibase_tenant_id(&mut both, &globals, &env).unwrap();
    assert_eq!(text(first.as_ref()).as_deref(), Some("tenant-call"));
    assert!(!both.contains_key("tenant_id"));
    assert!(both.contains_key("predibase_tenant_id"));
    let mut second = object(&[("predibase_tenant_id", param.clone())]);
    let popped = predibase_tenant_id(&mut second, &globals, &env).unwrap();
    assert_eq!(text(popped.as_ref()).as_deref(), Some("tenant-param"));
    assert!(second.is_empty());
    let mut nul = object(&[
        ("tenant_id", PromptJson::Null),
        ("predibase_tenant_id", param),
    ]);
    let after_null = predibase_tenant_id(&mut nul, &globals, &env).unwrap();
    assert_eq!(text(after_null.as_ref()).as_deref(), Some("tenant-param"));
}

#[test]
fn predibase_tenant_id_falls_back_to_its_global_then_the_variable() {
    let tenant = secret("tenant-global");
    let mut globals = empty_globals();
    globals.predibase_tenant_id = Some(&tenant);
    let env = environment(&[("PREDIBASE_TENANT_ID", "tenant-env")]);
    let global = predibase_tenant_id(&mut Object::new(), &globals, &env).unwrap();
    assert_eq!(text(global.as_ref()).as_deref(), Some("tenant-global"));
    globals.predibase_tenant_id = None;
    let variable = predibase_tenant_id(&mut Object::new(), &globals, &env).unwrap();
    assert_eq!(text(variable.as_ref()).as_deref(), Some("tenant-env"));
    let empty = environment(&[("PREDIBASE_TENANT_ID", "")]);
    assert!(
        predibase_tenant_id(&mut Object::new(), &globals, &empty)
            .unwrap()
            .is_none()
    );
    let unset = environment(&[]);
    assert!(
        predibase_tenant_id(&mut Object::new(), &globals, &unset)
            .unwrap()
            .is_none()
    );
}

#[test]
fn predibase_rejects_a_tenant_id_that_is_not_text() {
    let globals = empty_globals();
    let mut options = object(&[("tenant_id", PromptJson::Bool(true))]);
    let error = predibase_tenant_id(&mut options, &globals, &environment(&[])).unwrap_err();
    let HubDispatchError::FieldType { field } = &error else {
        panic!("a non-text tenant id is a field type error");
    };
    assert_eq!(*field, "tenant_id");
}

#[test]
fn predibase_base_pops_api_base_then_base_url() {
    let globals = empty_globals();
    let env = environment(&[("PREDIBASE_API_BASE", "http://env")]);
    let mut both = object(&[
        ("api_base", PromptJson::Text(String::from("http://param"))),
        ("base_url", PromptJson::Text(String::from("http://other"))),
    ]);
    let call = predibase_base(Some("http://call"), &globals, &env, &mut both).unwrap();
    assert_eq!(call.as_deref(), Some("http://call"));
    assert!(both.contains_key("api_base"));
    assert!(both.contains_key("base_url"));
    let param = predibase_base(None, &globals, &env, &mut both).unwrap();
    assert_eq!(param.as_deref(), Some("http://param"));
    assert!(!both.contains_key("api_base"));
    assert!(both.contains_key("base_url"));
    let other = predibase_base(None, &globals, &env, &mut both).unwrap();
    assert_eq!(other.as_deref(), Some("http://other"));
    assert!(both.is_empty());
}

#[test]
fn predibase_base_falls_back_to_the_global_then_the_variable() {
    let mut globals = empty_globals();
    let env = environment(&[("PREDIBASE_API_BASE", "http://env")]);
    let variable = predibase_base(None, &globals, &env, &mut Object::new()).unwrap();
    assert_eq!(variable.as_deref(), Some("http://env"));
    globals.api_base = Some("http://litellm");
    let global = predibase_base(None, &globals, &env, &mut Object::new()).unwrap();
    assert_eq!(global.as_deref(), Some("http://litellm"));
    globals.api_base = None;
    let empty = environment(&[("PREDIBASE_API_BASE", "")]);
    assert!(
        predibase_base(None, &globals, &empty, &mut Object::new())
            .unwrap()
            .is_none()
    );
    let unset = environment(&[]);
    assert!(
        predibase_base(None, &globals, &unset, &mut Object::new())
            .unwrap()
            .is_none()
    );
}

#[test]
fn predibase_key_puts_litellm_api_key_before_the_predibase_key() {
    let context = secret("ctx");
    let api = secret("api-global");
    let predibase = secret("predibase-global");
    let mut globals = empty_globals();
    globals.api_key = Some(&api);
    globals.predibase_key = Some(&predibase);
    let env = environment(&[("PREDIBASE_API_KEY", "predibase-env")]);
    assert_eq!(
        text(predibase_key(Some(&context), &globals, &env).as_ref()),
        Some(String::from("ctx"))
    );
    assert_eq!(
        text(predibase_key(None, &globals, &env).as_ref()),
        Some(String::from("api-global"))
    );
    globals.api_key = None;
    assert_eq!(
        text(predibase_key(None, &globals, &env).as_ref()),
        Some(String::from("predibase-global"))
    );
    globals.predibase_key = None;
    assert_eq!(
        text(predibase_key(None, &globals, &env).as_ref()),
        Some(String::from("predibase-env"))
    );
    assert!(predibase_key(None, &globals, &environment(&[])).is_none());
}
