#![cfg(test)]

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
    Globals, huggingface_chat_key, huggingface_embedding_key, huggingface_env, huggingface_hub_token,
    huggingface_rerank_base, huggingface_rerank_key, predibase_base, predibase_key,
    predibase_tenant_id, replicate_base, replicate_key,
};
use super::embedding::{EmbeddingCall, embed};
use super::hub::{HubGetAsync, PrefetchedTag};
use super::rerank::huggingface_rerank_config;
use super::tokens::Cl100k;
use super::transport::PostTransport;
use super::{
    CallLogging, ChatContext, DispatchDeps, HubCompletion, HubDispatchError, HubProvider,
    ReplicateServices, dispatch, huggingface, polling, predibase, select,
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
        Duration::default(),
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

#[test]
fn select_maps_only_the_three_hub_tags() {
    assert_eq!(
        select(ChatConfigTag::HuggingFaceChat),
        Some(HubProvider::HuggingFace)
    );
    assert_eq!(select(ChatConfigTag::Replicate), Some(HubProvider::Replicate));
    assert_eq!(select(ChatConfigTag::Predibase), Some(HubProvider::Predibase));
    assert_eq!(select(ChatConfigTag::OpenAiGpt), None);
    assert_eq!(select(ChatConfigTag::Anthropic), None);
}

#[tokio::test]
async fn dispatch_leaves_an_unowned_tag_alone() {
    let services = Services::idle();
    let globals = empty_globals();
    let environment = environment(&[]);
    let deps = services.deps(&globals, &environment);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context("m", &mut messages, &mut options, None, None);
    let completion = dispatch(&selection(ChatConfigTag::OpenAiGpt), &mut call, &deps, &mut log)
        .await
        .unwrap();
    assert!(completion.is_none());
    assert!(services.post.requests().is_empty());
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
fn replicate_key_and_base_follow_litellm_order() {
    let context = secret("ctx");
    let replicate_global = secret("replicate-global");
    let api = secret("api-global");
    let mut globals = empty_globals();
    globals.replicate_key = Some(&replicate_global);
    globals.api_key = Some(&api);
    let both = environment(&[
        ("REPLICATE_API_KEY", "replicate-env"),
        ("REPLICATE_API_TOKEN", "replicate-token"),
        ("REPLICATE_API_BASE", "http://replicate"),
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
    assert_eq!(replicate_base(Some("http://call"), &globals, &both), "http://call");
    globals.api_base = Some("http://global");
    assert_eq!(replicate_base(None, &globals, &both), "http://global");
    globals.api_base = None;
    assert_eq!(replicate_base(None, &globals, &both), "http://replicate");
    assert_eq!(
        replicate_base(None, &globals, &environment(&[])),
        "https://api.replicate.com/v1"
    );
}

#[test]
fn predibase_pops_tenant_id_before_its_global() {
    let tenant = secret("tenant-global");
    let mut globals = empty_globals();
    globals.predibase_tenant_id = Some(&tenant);
    let env = environment(&[("PREDIBASE_TENANT_ID", "tenant-env")]);
    let mut both = object(&[
        ("tenant_id", PromptJson::Text(String::from("tenant-call"))),
        (
            "predibase_tenant_id",
            PromptJson::Text(String::from("tenant-param")),
        ),
    ]);
    assert_eq!(
        text(predibase_tenant_id(&mut both, &globals, &env).unwrap().as_ref()),
        Some(String::from("tenant-call"))
    );
    assert!(!both.contains_key("tenant_id"));
    assert!(both.contains_key("predibase_tenant_id"));
    let mut second = object(&[(
        "predibase_tenant_id",
        PromptJson::Text(String::from("tenant-param")),
    )]);
    assert_eq!(
        text(predibase_tenant_id(&mut second, &globals, &env).unwrap().as_ref()),
        Some(String::from("tenant-param"))
    );
    assert!(second.is_empty());
    let mut nul = object(&[
        ("tenant_id", PromptJson::Null),
        (
            "predibase_tenant_id",
            PromptJson::Text(String::from("tenant-param")),
        ),
    ]);
    assert_eq!(
        text(predibase_tenant_id(&mut nul, &globals, &env).unwrap().as_ref()),
        Some(String::from("tenant-param"))
    );
    assert_eq!(
        text(predibase_tenant_id(&mut Object::new(), &globals, &env).unwrap().as_ref()),
        Some(String::from("tenant-global"))
    );
    globals.predibase_tenant_id = None;
    assert_eq!(
        text(predibase_tenant_id(&mut Object::new(), &globals, &env).unwrap().as_ref()),
        Some(String::from("tenant-env"))
    );
    assert!(predibase_tenant_id(&mut Object::new(), &globals, &environment(&[]))
        .unwrap()
        .is_none());
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
    assert_eq!(
        predibase_base(Some("http://call"), &globals, &env, &mut both)
            .unwrap()
            .as_deref(),
        Some("http://call")
    );
    assert!(both.contains_key("api_base"));
    assert!(both.contains_key("base_url"));
    assert_eq!(
        predibase_base(None, &globals, &env, &mut both)
            .unwrap()
            .as_deref(),
        Some("http://param")
    );
    assert!(!both.contains_key("api_base"));
    assert!(both.contains_key("base_url"));
    assert_eq!(
        predibase_base(None, &globals, &env, &mut both)
            .unwrap()
            .as_deref(),
        Some("http://other")
    );
    assert_eq!(
        predibase_base(None, &globals, &env, &mut Object::new())
            .unwrap()
            .as_deref(),
        Some("http://env")
    );
    let mut with_global = empty_globals();
    with_global.api_base = Some("http://litellm");
    assert_eq!(
        predibase_base(None, &with_global, &environment(&[]), &mut Object::new())
            .unwrap()
            .as_deref(),
        Some("http://litellm")
    );
    assert!(predibase_base(None, &globals, &environment(&[]), &mut Object::new())
        .unwrap()
        .is_none());
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

#[test]
fn the_pipeline_tag_source_reads_pipeline_tag_or_fails_500() {
    let found = PrefetchedTag(hub_reply(200, r#"{"pipeline_tag":"rerank"}"#));
    assert_eq!(
        found.tag("m", None).unwrap(),
        Some(String::from("rerank"))
    );
    let absent = PrefetchedTag(hub_reply(200, r#"{"id":"m"}"#));
    assert_eq!(absent.tag("m", None).unwrap(), None);
    let error = PrefetchedTag(hub_reply(200, "not json"))
        .tag("m", None)
        .unwrap_err();
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "the Hugging Face model info is not JSON");
}

#[test]
fn polling_reads_the_replicate_variables_and_reports_a_bad_one() {
    let env = environment(&[
        ("DEFAULT_REPLICATE_POLLING_RETRIES", "2"),
        ("DEFAULT_REPLICATE_POLLING_DELAY_SECONDS", "4"),
        ("REPLICATE_POLLING_DELAY_SECONDS", "0.25"),
        ("REPLICATE_MODEL_NAME_WITH_ID_LENGTH", "8"),
    ]);
    let found = polling(&env).unwrap();
    assert_eq!(found.retries, 2_i32);
    assert_eq!(found.retry_delay_seconds.to_bits(), 4.0_f64.to_bits());
    assert_eq!(found.polling_delay_seconds.to_bits(), 0.25_f64.to_bits());
    assert_eq!(found.model_name_with_id_length, 8);
    let defaults: ReplicatePolling = polling(&environment(&[])).unwrap();
    assert_eq!(defaults.retries, 5_i32);
    let error = polling(&environment(&[("DEFAULT_REPLICATE_POLLING_RETRIES", "five")]))
        .expect_err("a text retry count is an environment error");
    let HubDispatchError::Environment(source) = error else {
        panic!("a bad polling value is an environment error");
    };
    assert!(source.to_string().contains("DEFAULT_REPLICATE_POLLING_RETRIES"));
}

#[test]
fn the_cl100k_counter_counts_text() {
    let counter = Cl100k::new().unwrap();
    assert_eq!(counter.count("m", "hello world").unwrap(), 2);
    assert_eq!(counter.encode_ordinary("hello world"), 2);
}

#[tokio::test]
async fn the_embedding_arm_posts_the_prepared_body_and_logs_both_hooks() {
    let services = Services::new(
        ScriptedHub::new(Vec::new()),
        ScriptedPost::new(vec![http_reply(200, r#"[{"embedding":[0.5,0.25]}]"#)]),
        ScriptedTransport::new(Vec::new()),
    );
    let globals = empty_globals();
    let env = environment(&[("HUGGINGFACE_API_KEY", "hf-key")]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let input = vec![String::from("a"), String::from("b")];
    let options = object(&[
        (
            "input_type",
            PromptJson::Text(String::from("feature-extraction")),
        ),
        ("wait_for_model", PromptJson::Bool(true)),
    ]);
    let headers = Headers::from([(String::from("x-call"), String::from("1"))]);
    let call = EmbeddingCall {
        model: "org/model",
        input: &input,
        optional_params: &options,
        api_key: None,
        api_base: Some("http://mock/embed"),
        headers: &headers,
    };
    let response: EmbeddingResponse = embed(&call, &deps, &mut log).await.unwrap();
    assert_eq!(services.hub.calls(), 0);
    let requests = services.post.requests();
    assert_eq!(requests.len(), 1);
    let (url, sent_headers, sent_body) = requests.first().unwrap();
    assert_eq!(url, "http://mock/embed");
    assert_eq!(sent_headers.get("Authorization").unwrap(), "Bearer hf-key");
    assert_eq!(sent_headers.get("content-type").unwrap(), "application/json");
    assert_eq!(sent_headers.get("x-call").unwrap(), "1");
    assert_eq!(
        *sent_body,
        String::from(r#"{"inputs":["a","b"],"options":{"wait_for_model":true}}"#)
    );
    assert_eq!(response.data.len(), 1);
    assert_eq!(response.usage.prompt_tokens, 2);
    assert_eq!(
        state.model_call_details.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
    let Some(Json::List(items)) = state.model_call_details.get("original_response") else {
        panic!("the post_call keeps the reply");
    };
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn the_embedding_arm_reads_the_pipeline_tag_only_without_input_type() {
    let services = Services::new(
        ScriptedHub::new(vec![hub_reply(200, r#"{"pipeline_tag":"feature-extraction"}"#)]),
        ScriptedPost::new(vec![http_reply(200, r#"[{"embedding":[0.5]}]"#)]),
        ScriptedTransport::new(Vec::new()),
    );
    let globals = empty_globals();
    let env = environment(&[]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let input = vec![String::from("a")];
    let options = Object::new();
    let headers = Headers::new();
    let call = EmbeddingCall {
        model: "org/model",
        input: &input,
        optional_params: &options,
        api_key: None,
        api_base: None,
        headers: &headers,
    };
    embed(&call, &deps, &mut log).await.unwrap();
    let requests = services.hub.requests();
    assert_eq!(requests.len(), 1);
    let (url, sent_headers, query) = requests.first().unwrap();
    assert_eq!(url, "https://huggingface.co/api/models/org/model");
    assert!(sent_headers.is_empty());
    assert!(query.is_empty());
    let posted = services.post.requests();
    assert_eq!(
        posted.first().unwrap().0,
        "https://router.huggingface.co/hf-inference/pipeline/feature-extraction/org/model"
    );
}

#[tokio::test]
async fn the_embedding_arm_fails_with_500_when_the_reply_has_an_error_key() {
    let services = Services::new(
        ScriptedHub::new(Vec::new()),
        ScriptedPost::new(vec![http_reply(200, r#"{"error":"boom"}"#)]),
        ScriptedTransport::new(Vec::new()),
    );
    let globals = empty_globals();
    let env = environment(&[]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let input = vec![String::from("a")];
    let options = object(&[(
        "input_type",
        PromptJson::Text(String::from("feature-extraction")),
    )]);
    let headers = Headers::new();
    let call = EmbeddingCall {
        model: "org/model",
        input: &input,
        optional_params: &options,
        api_key: None,
        api_base: Some("http://mock/embed"),
        headers: &headers,
    };
    let error = embed(&call, &deps, &mut log)
        .await
        .expect_err("the error key fails the call");
    let HubDispatchError::Embedding(HuggingfaceEmbeddingError::Huggingface(error)) = error else {
        panic!("the error key is a Hugging Face error");
    };
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "boom");
}

#[tokio::test]
async fn the_huggingface_arm_prepares_the_chat_request() {
    let services = Services::idle();
    let globals = empty_globals();
    let env = environment(&[("HF_TOKEN", "hf-token")]);
    let deps = services.deps(&globals, &env);
    let mut messages = vec![user_message()];
    let mut options = object(&[("temperature", number(0)), ("stream", PromptJson::Bool(true))]);
    let headers = Headers::from([(String::from("x-call"), String::from("1"))]);
    let mut call = ChatContext {
        headers: Some(&headers),
        api_base: Some("http://mock/v1"),
        ..chat_context(
            "meta-llama/Llama-3-8B-Instruct",
            &mut messages,
            &mut options,
            Some("http://mock/v1"),
            None,
        )
    };
    let request = huggingface::chat_request(&mut call, &deps)
        .await
        .unwrap();
    let PreparedRequest::Stream(prepared) = request else {
        panic!("a stream call prepares a stream request");
    };
    assert_eq!(prepared.http.url.as_str(), "http://mock/v1/chat/completions");
    assert_eq!(
        prepared.http.headers.get("Authorization").unwrap(),
        "Bearer hf-token"
    );
    assert_eq!(
        prepared.http.headers.get("content-type").unwrap(),
        "application/json"
    );
    assert_eq!(prepared.http.headers.get("x-call").unwrap(), "1");
    let body: Object = serde_json::from_slice(&prepared.http.body).unwrap();
    assert_eq!(
        body.get("model"),
        Some(&PromptJson::Text(String::from(
            "meta-llama/Llama-3-8B-Instruct"
        )))
    );
    assert_eq!(body.get("stream"), Some(&PromptJson::Bool(true)));
    assert_eq!(body.get("temperature"), Some(&number(0)));
    assert_eq!(services.hub.calls(), 0);
    let mut plain_messages = vec![user_message()];
    let mut plain_options = Object::new();
    let mut plain = chat_context(
        "meta-llama/Llama-3-8B-Instruct",
        &mut plain_messages,
        &mut plain_options,
        Some("http://mock/v1"),
        None,
    );
    let plain_request = huggingface::chat_request(&mut plain, &deps)
        .await
        .unwrap();
    assert!(matches!(
        plain_request,
        PreparedRequest::Chat(_)
    ));
}

#[tokio::test]
async fn the_huggingface_arm_fetches_the_provider_mapping_without_a_base() {
    let mapping = r#"{"inferenceProviderMapping":{"hf-inference":{"status":"live","providerId":"meta-llama/Llama-3-8B-Instruct"}}}"#;
    let services = Services::new(
        ScriptedHub::new(vec![hub_reply(200, mapping)]),
        ScriptedPost::new(Vec::new()),
        ScriptedTransport::new(Vec::new()),
    );
    let globals = empty_globals();
    let env = environment(&[("HUGGINGFACE_API_KEY", "hub-secret")]);
    let deps = services.deps(&globals, &env);
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(
        "hf-inference/meta-llama/Llama-3-8B-Instruct",
        &mut messages,
        &mut options,
        None,
        None,
    );
    let request = huggingface::chat_request(&mut call, &deps)
        .await
        .unwrap();
    let PreparedRequest::Chat(prepared) = request else {
        panic!("a plain call prepares a chat request");
    };
    let requests = services.hub.requests();
    assert_eq!(requests.len(), 1);
    let (url, sent_headers, query) = requests.first().unwrap();
    assert_eq!(
        url,
        "https://huggingface.co/api/models/meta-llama/Llama-3-8B-Instruct"
    );
    assert_eq!(
        sent_headers.get("Authorization").unwrap(),
        "Bearer hub-secret"
    );
    assert_eq!(query.len(), 1);
    assert_eq!(query.first().unwrap().0, "expand");
    assert_eq!(query.first().unwrap().1, "inferenceProviderMapping");
    let body: Object = serde_json::from_slice(&prepared.http.body).unwrap();
    assert_eq!(
        body.get("model"),
        Some(&PromptJson::Text(String::from(
            "meta-llama/Llama-3-8B-Instruct"
        )))
    );
}

#[tokio::test]
async fn the_replicate_arm_creates_and_polls_with_the_strict_policy() {
    let services = Services::new(
        ScriptedHub::new(Vec::new()),
        ScriptedPost::new(Vec::new()),
        ScriptedTransport::new(vec![
            replicate_reply(200, CREATE),
            replicate_reply(200, DONE),
        ]),
    );
    let globals = empty_globals();
    let env = environment(&[]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let key = secret("ctx-key");
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(MODEL, &mut messages, &mut options, None, Some(&key));
    let completion = dispatch(
        &selection(ChatConfigTag::Replicate),
        &mut call,
        &deps,
        &mut log,
    )
    .await
    .unwrap();
    let Some(HubCompletion::Replicate(completion)) = completion else {
        panic!("the replicate tag returns a replicate completion");
    };
    let ReplicateCompletion::Reply(reply) = *completion else {
        panic!("a non-stream call returns the reply");
    };
    assert_eq!(reply.text, "ok");
    assert_eq!(reply.model, "replicate/meta-llama/llama-2-70b-chat");
    assert_eq!(reply.created, Some(NOW_SECONDS));
    assert_eq!(reply.usage.prompt_tokens, 1);
    assert_eq!(reply.usage.total_tokens, 3);
    let requests = services.transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests.first().unwrap().url,
        "https://api.replicate.com/v1/models/meta-llama/llama-2-70b-chat/predictions"
    );
    assert_eq!(
        requests.first().unwrap().headers.get("authorization").unwrap(),
        "Token ctx-key"
    );
    let sent: Object = serde_json::from_slice(
        requests.first().unwrap().body.as_deref().unwrap(),
    )
    .unwrap();
    assert_eq!(sent, serde_json::from_str::<Object>(PROMPT).unwrap());
    assert_eq!(requests.get(1).unwrap().url, "https://poll/1");
    assert_eq!(
        services.sleeper.delays().first().map(|delay| delay.to_bits()),
        Some(1.0_f64.to_bits())
    );
    assert!(state.model_call_details.get("log_event_type").is_none());
}

#[tokio::test]
async fn the_replicate_arm_uses_the_lenient_policy_for_an_async_call() {
    let services = Services::new(
        ScriptedHub::new(Vec::new()),
        ScriptedPost::new(Vec::new()),
        ScriptedTransport::new(vec![
            replicate_reply(200, CREATE),
            replicate_reply(200, r#"{"status":"succeeded","output":5}"#),
        ]),
    );
    let globals = empty_globals();
    let env = environment(&[]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = ChatContext {
        acompletion: true,
        ..chat_context(MODEL, &mut messages, &mut options, None, None)
    };
    let completion = dispatch(
        &selection(ChatConfigTag::Replicate),
        &mut call,
        &deps,
        &mut log,
    )
    .await
    .unwrap();
    let Some(HubCompletion::Replicate(completion)) = completion else {
        panic!("the replicate tag returns a replicate completion");
    };
    let ReplicateCompletion::Reply(reply) = *completion else {
        panic!("a non-stream call returns the reply");
    };
    assert_eq!(reply.text, "5");
    assert_eq!(reply.created, None);
}

#[tokio::test]
async fn the_replicate_arm_reports_a_missing_prediction_url() {
    let services = Services::new(
        ScriptedHub::new(Vec::new()),
        ScriptedPost::new(Vec::new()),
        ScriptedTransport::new(vec![replicate_reply(200, "{}")]),
    );
    let globals = empty_globals();
    let env = environment(&[]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(MODEL, &mut messages, &mut options, None, None);
    let error = match dispatch(
        &selection(ChatConfigTag::Replicate),
        &mut call,
        &deps,
        &mut log,
    )
    .await
    {
        Ok(_) => panic!("a create reply without a poll url fails"),
        Err(error) => error,
    };
    let HubDispatchError::Replicate(failure) = error else {
        panic!("the missing url is a replicate failure");
    };
    let Some(api) = failure.api() else {
        panic!("the missing url is a Replicate API error");
    };
    assert_eq!(api.status_code, 400);
    assert!(api.message.starts_with("LiteLLM Error - prediction url is None - "));
}

#[tokio::test]
async fn the_replicate_arm_logs_the_post_call_for_a_stream() {
    let services = Services::new(
        ScriptedHub::new(Vec::new()),
        ScriptedPost::new(Vec::new()),
        ScriptedTransport::new(vec![replicate_reply(200, CREATE)]),
    );
    let globals = empty_globals();
    let env = environment(&[]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let mut messages = vec![user_message()];
    let mut options = object(&[("stream", PromptJson::Bool(true))]);
    let mut call = chat_context(MODEL, &mut messages, &mut options, None, None);
    let completion = dispatch(
        &selection(ChatConfigTag::Replicate),
        &mut call,
        &deps,
        &mut log,
    )
    .await
    .unwrap();
    let Some(HubCompletion::Replicate(completion)) = completion else {
        panic!("the replicate tag returns a replicate completion");
    };
    assert!(matches!(*completion, ReplicateCompletion::Stream(_)));
    assert_eq!(
        state.model_call_details.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
    let Some(Json::List(input)) = state.model_call_details.get("input") else {
        panic!("the post_call keeps the messages");
    };
    assert_eq!(input.len(), 1);
}

#[tokio::test]
async fn the_predibase_arm_sends_the_generate_request_and_wraps_the_reply() {
    let services = Services::new(
        ScriptedHub::new(Vec::new()),
        ScriptedPost::new(vec![http_reply(200, r#"{"generated_text":"hello"}"#)]),
        ScriptedTransport::new(Vec::new()),
    );
    let globals = empty_globals();
    let env = environment(&[("PREDIBASE_API_KEY", "pb-env")]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let mut messages = vec![user_message()];
    let mut options = object(&[(
        "tenant_id",
        PromptJson::Text(String::from("tenant-1")),
    )]);
    let mut call = ChatContext {
        acompletion: true,
        ..chat_context(
            "meta-llama/llama-2-7b-chat",
            &mut messages,
            &mut options,
            Some("http://mock"),
            None,
        )
    };
    let completion = dispatch(
        &selection(ChatConfigTag::Predibase),
        &mut call,
        &deps,
        &mut log,
    )
    .await
    .unwrap();
    let Some(HubCompletion::Predibase(completion)) = completion else {
        panic!("the predibase tag returns a predibase completion");
    };
    let predibase::Completion::Reply(reply) = *completion else {
        panic!("a non-stream call returns the reply");
    };
    assert_eq!(reply.model, "meta-llama/llama-2-7b-chat");
    assert_eq!(reply.created, NOW_SECONDS);
    assert_eq!(
        reply.choices.first().and_then(|choice| choice.content.as_deref()),
        Some("hello")
    );
    let requests = services.post.requests();
    assert_eq!(requests.len(), 1);
    let (url, sent_headers, sent_body) = requests.first().unwrap();
    assert_eq!(
        url,
        "http://mock/tenant-1/deployments/v2/llms/meta-llama/llama-2-7b-chat/generate"
    );
    assert_eq!(sent_headers.get("Authorization").unwrap(), "Bearer pb-env");
    assert_eq!(sent_headers.get("content-type").unwrap(), "application/json");
    let body: Object = serde_json::from_str(sent_body).unwrap();
    assert_eq!(
        body.get("inputs"),
        Some(&PromptJson::Text(String::from("<s>[INST] hi [/INST]\n")))
    );
    assert_eq!(
        body.get("parameters"),
        Some(&PromptJson::Object(object(&[
            ("details", PromptJson::Bool(true)),
            ("max_new_tokens", number(4096)),
            ("return_full_text", PromptJson::Bool(false)),
        ])))
    );
    assert_eq!(
        state.model_call_details.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
}

#[tokio::test]
async fn the_predibase_arm_fails_with_the_tenant_id_text() {
    let services = Services::idle();
    let globals = empty_globals();
    let env = environment(&[("PREDIBASE_API_KEY", "pb-env")]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context("m", &mut messages, &mut options, Some("http://mock"), None);
    let error = match dispatch(
        &selection(ChatConfigTag::Predibase),
        &mut call,
        &deps,
        &mut log,
    )
    .await
    {
        Ok(_) => panic!("a missing tenant id fails the call"),
        Err(error) => error,
    };
    let HubDispatchError::PredibaseSetup(PredibaseSetupError::MissingTenantId) = &error else {
        panic!("the missing tenant id is a setup error");
    };
    assert_eq!(
        error.to_string(),
        "Missing Predibase Tenant ID - Required for making the request. Set dynamically (e.g. `completion(..tenant_id=<MY-ID>)`) or in env - `PREDIBASE_TENANT_ID`."
    );
    assert!(services.post.requests().is_empty());
}

#[tokio::test]
async fn the_predibase_arm_streams_with_the_stream_suffix_and_body_flag() {
    let services = Services::idle();
    let globals = empty_globals();
    let env = environment(&[("PREDIBASE_API_KEY", "pb-env")]);
    let deps = services.deps(&globals, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, log_env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &log_env,
    };
    let mut messages = vec![user_message()];
    let mut options = object(&[
        ("stream", PromptJson::Bool(true)),
        ("tenant_id", PromptJson::Text(String::from("tenant-1"))),
    ]);
    let mut call = ChatContext {
        acompletion: true,
        ..chat_context("m", &mut messages, &mut options, Some("http://mock"), None)
    };
    let error = match dispatch(
        &selection(ChatConfigTag::Predibase),
        &mut call,
        &deps,
        &mut log,
    )
    .await
    {
        Ok(_) => panic!("the scripted stream transport fails"),
        Err(error) => error,
    };
    let HubDispatchError::Operation(OperationError::UnsupportedOperation { provider, .. }) = error
    else {
        panic!("the stream error comes from the transport");
    };
    assert_eq!(provider, "predibase");
    let streams = services.post.stream_requests();
    assert_eq!(streams.len(), 1);
    let (url, _, sent_body) = streams.first().unwrap();
    assert_eq!(
        url,
        "http://mock/tenant-1/deployments/v2/llms/m/generate_stream"
    );
    let body: Object = serde_json::from_str(sent_body).unwrap();
    assert_eq!(body.get("stream"), Some(&PromptJson::Bool(true)));
    let Some(PromptJson::Object(parameters)) = body.get("parameters") else {
        panic!("the request keeps its parameters");
    };
    assert!(!parameters.contains_key("stream"));
    assert!(services.post.requests().is_empty());
}

#[test]
fn the_rerank_config_takes_its_key_and_base_from_the_chains() {
    let hf_global = secret("hf-global");
    let mut globals = empty_globals();
    globals.huggingface_key = Some(&hf_global);
    let env = environment(&[
        ("HUGGINGFACE_API_KEY", "hf-api"),
        ("HF_API_BASE", "http://hf"),
    ]);
    let call_key = secret("call");
    let explicit = huggingface_rerank_config(Some(&call_key), Some("http://call"), &globals, &env)
        .expect("the rerank config builds");
    assert_eq!(text(explicit.key.as_ref()), Some(String::from("call")));
    assert_eq!(explicit.api_base.as_deref(), Some("http://call"));
    let inherited = huggingface_rerank_config(None, None, &globals, &env)
        .expect("the rerank config builds from the chains");
    assert_eq!(text(inherited.key.as_ref()), Some(String::from("hf-global")));
    assert_eq!(inherited.api_base.as_deref(), Some("http://hf"));
    assert_eq!(
        inherited.get_complete_url(None, "m", None),
        "http://hf/rerank"
    );
    assert_eq!(
        inherited.get_complete_url(Some("http://call"), "m", None),
        "http://call/rerank"
    );
    let headers = inherited
        .validate_environment(EnvironmentRequest {
            headers: &Headers::new(),
            model: "m",
            api_key: None,
            optional_params: None,
            litellm_params: None,
        })
        .expect("the headers build");
    assert_eq!(
        headers.get("Authorization").unwrap(),
        "Bearer hf-global"
    );
    let supported = inherited.supported_rerank_params("m");
    assert!(supported.contains(&String::from("query")));
}
