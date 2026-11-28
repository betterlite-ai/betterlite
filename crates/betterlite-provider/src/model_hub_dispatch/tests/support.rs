use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::chat_config_selection::tags::{
    ChatConfigTag, ConfigConstruction, ConfigSelection,
};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{StreamResponse, UpstreamResponse};
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::huggingface_chat::mapping::HubResponse;
use betterlite_openai::huggingface_embedding::response::{EmbeddingResponse, OrdinaryEncoder};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::count::error::CountError;
use chrono::{DateTime, Utc};
use futures_util::FutureExt as _;
use futures_util::future::BoxFuture;
use http::HeaderMap;

use super::super::credentials::Globals;
use super::super::embedding::{EmbeddingCall, embed};
use super::super::transport::{HubGetAsync, PostTransport, WireRequest};
use super::super::{
    ChatContext, DispatchDeps, HubCompletion, HubDispatchError, ReplicateServices, dispatch,
};
use crate::oci_common::Clock;
use crate::operation::OperationError;
use crate::predibase::reply::TokenCounter as PredibaseTokenCounter;
use crate::replicate::poll::ReplicateSleeper;
use crate::replicate::poll::transport::{ReplicateRequest, ReplicateResponse, ReplicateTransport};
use crate::replicate::reply::{PostCall, PostCallLogger, TokenCounter as ReplicateTokenCounter};

pub(super) type Env = BTreeMap<String, String>;
pub(super) type Outcome<T> = (Result<T, HubDispatchError>, LoggingState);

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
pub(super) const NOW_SECONDS: i64 = 1_700_000_000;

pub(super) fn environment(pairs: &[(&str, &str)]) -> Env {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn empty_globals<'source>() -> Globals<'source> {
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

pub(super) fn object(pairs: &[(&str, PromptJson)]) -> Object {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) fn user_message() -> Message {
    serde_json::from_str(r#"{"role":"user","content":"hi"}"#).unwrap()
}

pub(super) fn chat_context<'source>(
    model: &'source str,
    messages: &'source mut Vec<Message>,
    options: &'source mut Object,
    api_base: Option<&'source str>,
) -> ChatContext<'source> {
    ChatContext {
        model,
        messages,
        optional_params: options,
        api_key: None,
        api_base,
        headers: None,
        custom_prompt_dict: None,
        acompletion: false,
    }
}

pub(super) fn http_reply(status: u16, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub(super) fn hub_reply(body: &str) -> HubResponse {
    HubResponse {
        status_code: 200,
        headers: Headers::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub(super) fn replicate_reply(body: &str) -> ReplicateResponse {
    ReplicateResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

#[derive(Debug, Clone)]
pub(super) struct Recorded {
    pub(super) method: String,
    pub(super) url: String,
    pub(super) query: Vec<(String, String)>,
    pub(super) headers: Headers,
    pub(super) body: String,
}

// answers the Hub GETs and the provider POSTs from scripted replies and records every request
pub(super) struct ScriptedHttp {
    hub_replies: Mutex<VecDeque<HubResponse>>,
    post_replies: Mutex<VecDeque<UpstreamResponse>>,
    recorded: Mutex<Vec<Recorded>>,
}

impl ScriptedHttp {
    fn record(&self, request: &WireRequest<'_>) {
        let body = String::from_utf8(request.body.unwrap_or_default().to_vec()).unwrap();
        let query = request
            .query
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        self.recorded.lock().unwrap().push(Recorded {
            method: request.method.to_string(),
            url: request.url.to_owned(),
            query,
            headers: request.headers.clone(),
            body,
        });
    }

    fn answer<T>(
        &self,
        request: &WireRequest<'_>,
        replies: &Mutex<VecDeque<T>>,
    ) -> Result<T, HubDispatchError> {
        self.record(request);
        replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or(HubDispatchError::BodyNotObject)
    }

    pub(super) fn sent(&self, method: &str) -> Vec<Recorded> {
        self.recorded
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.method == method)
            .cloned()
            .collect()
    }
}

#[async_trait]
impl HubGetAsync for ScriptedHttp {
    async fn get(&self, request: &WireRequest<'_>) -> Result<HubResponse, HubDispatchError> {
        self.answer(request, &self.hub_replies)
    }
}

#[async_trait]
impl PostTransport for ScriptedHttp {
    async fn post(&self, request: &WireRequest<'_>) -> Result<UpstreamResponse, HubDispatchError> {
        self.answer(request, &self.post_replies)
    }

    async fn post_stream(
        &self,
        request: &WireRequest<'_>,
    ) -> Result<StreamResponse, HubDispatchError> {
        self.record(request);
        let operation = String::from("stream");
        let provider = String::from("scripted");
        Err(HubDispatchError::Operation(
            OperationError::UnsupportedOperation {
                provider,
                operation,
            },
        ))
    }
}

pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<ReplicateResponse>>,
    pub(super) sent: Mutex<Vec<ReplicateRequest>>,
}

#[async_trait]
impl ReplicateTransport for ScriptedTransport {
    async fn send(&self, request: ReplicateRequest) -> Result<ReplicateResponse, TransportError> {
        self.sent.lock().unwrap().push(request);
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(TransportError::Closed)
    }
}

pub(super) struct RecordingSleeper {
    pub(super) delays: Mutex<Vec<f64>>,
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
        DateTime::from_timestamp(NOW_SECONDS, 0).unwrap()
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

#[async_trait]
impl PredibaseTokenCounter for Counting {
    async fn count_messages(&self, messages: &[Message]) -> Option<i64> {
        i64::try_from(messages.len()).ok()
    }

    async fn count_text(&self, text: &str) -> Option<i64> {
        i64::try_from(text.chars().count()).ok()
    }
}

struct Discard;

impl LogSink for Discard {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) struct Services {
    pub(super) http: ScriptedHttp,
    pub(super) transport: Arc<ScriptedTransport>,
    pub(super) sleeper: Arc<RecordingSleeper>,
    logger: SilentLogger,
    prompts: PromptFactory,
    counter: Counting,
    encoder: LengthEncoder,
    clock: FixedClock,
    redactor: Redactor,
}

impl Services {
    pub(super) fn new(
        hub: Vec<HubResponse>,
        post: Vec<UpstreamResponse>,
        replicate: Vec<ReplicateResponse>,
    ) -> Self {
        Self {
            http: ScriptedHttp {
                hub_replies: Mutex::new(VecDeque::from(hub)),
                post_replies: Mutex::new(VecDeque::from(post)),
                recorded: Mutex::new(Vec::new()),
            },
            transport: Arc::new(ScriptedTransport {
                replies: Mutex::new(VecDeque::from(replicate)),
                sent: Mutex::new(Vec::new()),
            }),
            sleeper: Arc::new(RecordingSleeper {
                delays: Mutex::new(Vec::new()),
            }),
            logger: SilentLogger,
            prompts: PromptFactory {
                cache: Mutex::new(builtin_entries()),
                hub: Hub::Unreachable,
            },
            counter: Counting,
            encoder: LengthEncoder,
            clock: FixedClock,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
        }
    }

    pub(super) fn deps<'deps>(
        &'deps self,
        globals: &'deps Globals<'deps>,
        environment: &'deps Env,
    ) -> DispatchDeps<'deps> {
        DispatchDeps {
            globals,
            environment,
            hub: &self.http,
            post: &self.http,
            encoder: &self.encoder,
            replicate: ReplicateServices {
                transport: Arc::<ScriptedTransport>::clone(&self.transport),
                sleeper: Arc::<RecordingSleeper>::clone(&self.sleeper),
                logger: &self.logger,
            },
            prompts: &self.prompts,
            replicate_counter: &self.counter,
            predibase_counter: &self.counter,
            clock: &self.clock,
            default_max_tokens: 4096,
            now: self.clock.now(),
            timeout: Duration::from_secs(5),
        }
    }

    // raw request logging is on so the pre_call record of the request headers stays visible after post_call
    pub(super) fn logging(&self) -> (LoggingState, PrePostEnv<'_>) {
        let state = LoggingState::new(
            serde_json::from_str::<LoggingInit>(INIT).unwrap(),
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &self.redactor,
            || "trace".to_owned(),
            Duration::default,
        )
        .unwrap();
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags {
                log_raw_request_response: true,
                turn_off_message_logging: false,
            },
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &Discard,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap(),
        };
        (state, env)
    }
}

type Deps<'call> = &'call DispatchDeps<'call>;
type Log<'call, 'state, 'env> = &'call mut LogContext<'state, 'env>;
type Ran<'call, T> = BoxFuture<'call, Result<T, HubDispatchError>>;

// one entry point of the module under test
pub(super) trait Arm {
    type Output;

    fn run<'call>(self, deps: Deps<'call>, log: Log<'call, '_, '_>) -> Ran<'call, Self::Output>
    where
        Self: 'call;
}

impl Arm for &EmbeddingCall<'_> {
    type Output = EmbeddingResponse;

    fn run<'call>(self, deps: Deps<'call>, log: Log<'call, '_, '_>) -> Ran<'call, Self::Output>
    where
        Self: 'call,
    {
        embed(self, deps, log).boxed()
    }
}

impl Arm for (ChatConfigTag, &mut ChatContext<'_>) {
    type Output = Option<HubCompletion>;

    fn run<'call>(self, deps: Deps<'call>, log: Log<'call, '_, '_>) -> Ran<'call, Self::Output>
    where
        Self: 'call,
    {
        let (tag, call) = self;
        let selection = ConfigSelection {
            tag,
            construction: ConfigConstruction::Shared,
        };
        dispatch(&selection, call, deps, log).boxed()
    }
}

pub(super) fn run_with<'run, A>(
    services: &'run Services,
    globals: &'run Globals<'_>,
    env: &'run Env,
    arm: A,
) -> BoxFuture<'run, Outcome<A::Output>>
where
    A: Arm + Send + 'run,
    A::Output: 'run,
{
    Box::pin(async move {
        let deps = services.deps(globals, env);
        let (mut state, log_env) = services.logging();
        let mut log = LogContext {
            state: &mut state,
            env: &log_env,
        };
        let outcome = arm.run(&deps, &mut log).await;
        (outcome, state)
    })
}
