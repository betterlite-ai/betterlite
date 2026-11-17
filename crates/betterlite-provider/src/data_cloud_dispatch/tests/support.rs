use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use secrecy::SecretString;

use crate::data_cloud_dispatch::call::CompletionCall;
use crate::data_cloud_dispatch::context::{DispatchContext, DispatchGlobals, EnvironmentSource};
use crate::data_cloud_dispatch::error::DispatchError;
use crate::data_cloud_dispatch::handler::{ChatCompletionHandler, HandlerError};
use crate::data_cloud_dispatch::transport::{PostTransport, StreamHead, WireRequest};
use crate::databricks_auth::oauth::{OauthReply, OauthTokenRequest, OauthTokenSource};
use crate::oci_common::Clock;

use super::lookup::secrets;

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
pub(super) const TEST_KEY: &str = include_str!("../../oci_common/fixtures/test-private.pem");

struct Discard;

impl LogSink for Discard {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn logging(redactor: &Redactor) -> (LoggingState, PrePostEnv<'_>) {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).expect("the fixture init parses"),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "trace".to_owned(),
        Duration::default,
    )
    .expect("the fixture logging state builds");
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
        redactor,
        log: &Discard,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
            .expect("the fixture time parses"),
    };
    (state, env)
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the fixture redactor builds")
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn user_message() -> Message {
    serde_json::from_str(r#"{"role":"user","content":"hi"}"#).expect("the fixture message parses")
}

pub(super) struct Fixture {
    pub(super) secrets: SecretLookup,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) model: String,
    pub(super) messages: Vec<Message>,
    pub(super) optional_params: Object,
    pub(super) litellm_params: Object,
    pub(super) provider: String,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
    pub(super) headers: Option<BTreeMap<String, String>>,
    pub(super) provider_config: Option<ChatConfigTag>,
    pub(super) stream: Option<bool>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self {
            secrets: secrets(),
            environment: BTreeMap::new(),
            model: String::from("databricks-meta-llama-3-3-70b-instruct"),
            messages: vec![user_message()],
            optional_params: Object::new(),
            litellm_params: Object::new(),
            provider: String::from("databricks"),
            api_base: None,
            api_key: None,
            headers: None,
            provider_config: None,
            stream: None,
        }
    }

    pub(super) fn context<'fixture>(
        &'fixture self,
        globals: DispatchGlobals<'fixture>,
    ) -> DispatchContext<'fixture> {
        DispatchContext {
            acompletion: false,
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            custom_llm_provider: &self.provider,
            headers: self.headers.as_ref(),
            litellm_params: &self.litellm_params,
            messages: &self.messages,
            model: &self.model,
            optional_params: &self.optional_params,
            provider_config: self.provider_config,
            stream: self.stream,
            timeout: None,
            globals,
            env: EnvironmentSource {
                secrets: &self.secrets,
                environment: &self.environment,
            },
        }
    }
}

pub(super) struct Recorder {
    calls: Mutex<Vec<CompletionCall>>,
    failure: Option<u16>,
}

impl Recorder {
    pub(super) const fn succeeding() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: None,
        }
    }

    pub(super) const fn failing(status_code: u16) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: Some(status_code),
        }
    }

    pub(super) fn calls(&self) -> Vec<CompletionCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ChatCompletionHandler for Recorder {
    type Reply = String;

    fn complete(&self, call: &CompletionCall) -> Result<Self::Reply, HandlerError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.clone());
        self.failure.map_or_else(
            || Ok(String::from("reply")),
            |status_code| Err(HandlerError::Upstream { status_code }),
        )
    }
}

#[derive(Clone)]
pub(super) struct Sent {
    pub(super) method: String,
    pub(super) url: String,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
    pub(super) deadline: Duration,
}

#[derive(Default)]
pub(super) struct Scripted {
    replies: Mutex<VecDeque<UpstreamResponse>>,
    streams: Mutex<VecDeque<StreamHead>>,
    recorded: Mutex<Vec<Sent>>,
}

impl Scripted {
    pub(super) fn with_replies(replies: Vec<UpstreamResponse>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            streams: Mutex::new(VecDeque::new()),
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn with_stream(head: StreamHead) -> Self {
        Self {
            replies: Mutex::new(VecDeque::new()),
            streams: Mutex::new(VecDeque::from([head])),
            recorded: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, request: &WireRequest<'_>) {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Sent {
                method: request.method.to_string(),
                url: request.url.to_owned(),
                headers: request.headers.clone(),
                body: request.body.to_vec(),
                deadline: request.deadline,
            });
    }

    pub(super) fn sent(&self) -> Vec<Sent> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn no_reply() -> DispatchError {
    DispatchError::Upstream {
        status: 599,
        body: String::from("the fixture holds no reply"),
    }
}

fn take<T>(queue: &Mutex<VecDeque<T>>) -> Result<T, DispatchError> {
    queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pop_front()
        .ok_or_else(no_reply)
}

#[async_trait]
impl PostTransport for Scripted {
    async fn post(&self, request: &WireRequest<'_>) -> Result<UpstreamResponse, DispatchError> {
        self.record(request);
        take(&self.replies)
    }

    async fn post_stream(&self, request: &WireRequest<'_>) -> Result<StreamHead, DispatchError> {
        self.record(request);
        take(&self.streams)
    }
}

pub(super) struct NoTokens;

#[async_trait]
impl OauthTokenSource for NoTokens {
    async fn send(&self, _request: &OauthTokenRequest) -> OauthReply {
        OauthReply::Transport(String::from("the fixture holds no token source"))
    }
}

pub(super) struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("the fixture time converts")
    }
}

pub(super) fn ok(body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub(super) fn status(code: u16, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status: code,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub(super) fn databricks_embedding_reply() -> &'static str {
    r#"{"object":"list","data":[{"object":"embedding","embedding":[0.1,0.2],"index":0}],"model":"databricks-bge-large-en","usage":{"prompt_tokens":2,"total_tokens":2}}"#
}
