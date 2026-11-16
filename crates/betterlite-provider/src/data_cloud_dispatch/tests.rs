#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_context::resource_config_selection::responses::{
    AzureResponsesFacts, MantleResponses, ResponsesModelFacts, select_responses_config,
};
use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_prompts::message_content::Message;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use chrono::{DateTime, Utc};
use futures_util::stream::{self, StreamExt as _};
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::call::{CompletionCall, EmbeddingCall, EmbeddingReply};
use super::chat::{complete_databricks, dispatch};
use super::context::{CallLogging, DispatchContext, DispatchGlobals};
use super::embed::{EmbeddingContext, dispatch as embedding_dispatch};
use super::error::DispatchError;
use super::handler::{ChatCompletionHandler, HandlerError};
use super::oauth::OauthM2m;
use super::oci_stream::{OciStreamRequest, open_stream};
use super::select::{EmbeddingConfig, embedding_config};
use super::transport::{PostTransport, StreamHead, WireRequest};
use crate::databricks_auth::oauth::{
    oauth_token_request, parse_oauth_token_reply, request_oauth_token,
};
use crate::oci_chat::stream::StreamItem;
use crate::oci_common::Clock;
use crate::snowflake::embedding::transform_embedding_response;

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const TEST_KEY: &str = include_str!("../oci_common/fixtures/test-private.pem");

struct Offline;

impl OidcHttp for Offline {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the dispatch fixture holds no OIDC transport"),
        })
    }
}

impl AzureOidcTokens for Offline {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Err(AzureOidcError::Provider {
            message: String::from("the dispatch fixture holds no Azure token source"),
        })
    }
}

impl OidcClock for Offline {
    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(Offline),
        azure_tokens: Arc::new(Offline),
        clock: Arc::new(Offline),
        oidc_cache: OidcTokenCache::default(),
    }
}

struct Discard;

impl LogSink for Discard {
    fn emit(&self, _record: LogRecord) {}
}

fn logging(redactor: &Redactor) -> (LoggingState, PrePostEnv<'_>) {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).expect("the fixture init parses"),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "trace".to_owned(),
        Duration::default(),
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

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the fixture redactor builds")
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn user_message() -> Message {
    serde_json::from_str(r#"{"role":"user","content":"hi"}"#).expect("the fixture message parses")
}

#[derive(Clone)]
struct OwnedGlobals {
    api_base: Option<String>,
    api_key: Option<SecretString>,
    databricks_key: Option<SecretString>,
    headers: Option<BTreeMap<String, String>>,
}

impl OwnedGlobals {
    const fn empty() -> Self {
        Self {
            api_base: None,
            api_key: None,
            databricks_key: None,
            headers: None,
        }
    }

    fn borrowed(&self) -> DispatchGlobals<'_> {
        DispatchGlobals {
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            databricks_key: self.databricks_key.as_ref(),
            headers: self.headers.as_ref(),
        }
    }
}

struct Fixture {
    secrets: SecretLookup,
    environment: BTreeMap<String, String>,
    globals: OwnedGlobals,
    model: String,
    messages: Vec<Message>,
    optional_params: Object,
    litellm_params: Object,
    provider: String,
    api_base: Option<String>,
    api_key: Option<SecretString>,
    headers: Option<BTreeMap<String, String>>,
    stream: Option<bool>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            secrets: secrets(),
            environment: BTreeMap::new(),
            globals: OwnedGlobals::empty(),
            model: String::from("databricks-meta-llama-3-3-70b-instruct"),
            messages: vec![user_message()],
            optional_params: Object::new(),
            litellm_params: Object::new(),
            provider: String::from("databricks"),
            api_base: None,
            api_key: None,
            headers: None,
            stream: None,
        }
    }

    fn context<'fixture>(
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
            provider_config: None,
            stream: self.stream,
            timeout: None,
            globals,
            secrets: &self.secrets,
            environment: &self.environment,
        }
    }

    fn embedding_call(&self) -> EmbeddingCall {
        EmbeddingCall {
            model: self.model.clone(),
            input: PromptJson::Text(String::from("hello")),
            optional_params: self.optional_params.clone(),
            litellm_params: Object::new(),
            api_key: self.api_key.clone(),
            api_base: self.api_base.clone(),
            headers: self.headers.clone(),
            timeout: None,
        }
    }
}

struct Recorder {
    calls: Mutex<Vec<CompletionCall>>,
    failure: Option<u16>,
}

impl Recorder {
    const fn succeeding() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: None,
        }
    }

    const fn failing(status_code: u16) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: Some(status_code),
        }
    }

    fn calls(&self) -> Vec<CompletionCall> {
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
        match self.failure {
            None => Ok(String::from("reply")),
            Some(status_code) => Err(HandlerError::Upstream { status_code }),
        }
    }
}

#[derive(Clone)]
struct Sent {
    method: String,
    url: String,
    headers: HeaderMap,
    body: Vec<u8>,
    deadline: Duration,
}

#[derive(Default)]
struct Scripted {
    replies: Mutex<VecDeque<UpstreamResponse>>,
    streams: Mutex<VecDeque<StreamHead>>,
    recorded: Mutex<Vec<Sent>>,
}

impl Scripted {
    fn with_replies(replies: Vec<UpstreamResponse>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            ..Self::default()
        }
    }

    fn with_stream(head: StreamHead) -> Self {
        Self {
            streams: Mutex::new(VecDeque::from([head])),
            ..Self::default()
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

    fn sent(&self) -> Vec<Sent> {
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

#[async_trait]
impl PostTransport for Scripted {
    async fn post(&self, request: &WireRequest<'_>) -> Result<UpstreamResponse, DispatchError> {
        self.record(request);
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(no_reply)
    }

    async fn post_stream(&self, request: &WireRequest<'_>) -> Result<StreamHead, DispatchError> {
        self.record(request);
        self.streams
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(no_reply)
    }
}

struct NoTokens;

#[async_trait]
impl crate::databricks_auth::oauth::OauthTokenSource for NoTokens {
    async fn send(
        &self,
        _request: &crate::databricks_auth::oauth::OauthTokenRequest,
    ) -> crate::databricks_auth::oauth::OauthReply {
        crate::databricks_auth::oauth::OauthReply::Transport(String::from(
            "the fixture holds no token source",
        ))
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("the fixture time converts")
    }
}

fn ok(body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn status(code: u16, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status: code,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn embedding_context<'context>(
    fixture: &'context Fixture,
    globals: DispatchGlobals<'context>,
    transport: &'context Scripted,
    tokens: &'context NoTokens,
    clock: &'context FixedClock,
) -> EmbeddingContext<'context> {
    EmbeddingContext {
        globals,
        secrets: &fixture.secrets,
        environment: &fixture.environment,
        transport,
        tokens,
        clock,
    }
}

fn databricks_embedding_reply() -> &'static str {
    r#"{"object":"list","data":[{"object":"embedding","embedding":[0.1,0.2],"index":0}],"model":"databricks-bge-large-en","usage":{"prompt_tokens":2,"total_tokens":2}}"#
}

#[test]
fn databricks_completion_prefers_the_context_then_the_globals() {
    let mut fixture = Fixture::new();
    fixture.environment = environment(&[
        ("DATABRICKS_API_BASE", "https://env.example.com"),
        ("DATABRICKS_API_KEY", "env-key"),
    ]);
    fixture.api_base = Some(String::from("https://context.example.com"));
    fixture.api_key = Some(SecretString::from("context-key"));
    fixture.headers = Some(BTreeMap::from([(
        String::from("X-Context"),
        String::from("one"),
    )]));
    let mut globals = OwnedGlobals::empty();
    globals.api_base = Some(String::from("https://global.example.com"));
    globals.api_key = Some(SecretString::from("global-key"));
    globals.databricks_key = Some(SecretString::from("databricks-key"));
    let handler = Recorder::succeeding();
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let outcome = dispatch(
        ChatConfigTag::Databricks,
        &fixture.context(globals.borrowed()),
        &handler,
        &mut log,
    )
    .expect("the databricks arm answers");
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    let calls = handler.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(
        call.api_base.as_deref(),
        Some("https://context.example.com")
    );
    assert_eq!(
        call.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some(&String::from("context-key"))
    );
    assert_eq!(
        call.headers
            .as_ref()
            .and_then(|headers| headers.get("X-Context")),
        Some(&String::from("one"))
    );
    assert_eq!(call.custom_llm_provider, "databricks");
}

#[test]
fn databricks_completion_reads_the_environment_when_no_setting_is_given() {
    let mut fixture = Fixture::new();
    fixture.environment = environment(&[
        ("DATABRICKS_API_BASE", "https://env.example.com"),
        ("DATABRICKS_API_KEY", "env-key"),
    ]);
    let handler = Recorder::succeeding();
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let outcome = complete_databricks(
        &fixture.context(DispatchGlobals::default()),
        &handler,
        &mut log,
    );
    assert!(outcome.is_ok());
    let calls = handler.calls();
    assert_eq!(
        calls[0].api_base.as_deref(),
        Some("https://env.example.com")
    );
    assert_eq!(
        calls[0].api_key.as_ref().map(ExposeSecret::expose_secret),
        Some(&String::from("env-key"))
    );
}

#[test]
fn databricks_completion_prefers_the_global_key_over_the_databricks_key() {
    let mut fixture = Fixture::new();
    fixture.environment = environment(&[("DATABRICKS_API_KEY", "env-key")]);
    let mut globals = OwnedGlobals::empty();
    globals.api_key = Some(SecretString::from("global-key"));
    globals.databricks_key = Some(SecretString::from("databricks-key"));
    let handler = Recorder::succeeding();
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let outcome = complete_databricks(&fixture.context(globals.borrowed()), &handler, &mut log);
    assert!(outcome.is_ok());
    assert_eq!(
        handler.calls()[0]
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some(&String::from("global-key"))
    );
}

#[test]
fn databricks_completion_logs_a_failure_and_returns_the_same_error() {
    let fixture = Fixture::new();
    let handler = Recorder::failing(500);
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let outcome = complete_databricks(
        &fixture.context(DispatchGlobals::default()),
        &handler,
        &mut log,
    );
    match outcome {
        Err(DispatchError::Handler(HandlerError::Upstream { status_code })) => {
            assert_eq!(status_code, 500);
        }
        other => panic!("expected the handler failure, got {other:?}"),
    }
    let details = &state.model_call_details;
    assert_eq!(
        details.get("original_response"),
        Some(&Json::Text(String::from(
            "the provider answered with status 500"
        )))
    );
    assert_eq!(
        details.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
    assert!(matches!(
        details.get("additional_args"),
        Some(Json::Object(fields)) if fields.contains_key("headers")
    ));
}

#[test]
fn databricks_completion_without_a_stream_logs_nothing_on_success() {
    let fixture = Fixture::new();
    let handler = Recorder::succeeding();
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let outcome = complete_databricks(
        &fixture.context(DispatchGlobals::default()),
        &handler,
        &mut log,
    );
    assert!(outcome.is_ok());
    assert!(state.model_call_details.get("original_response").is_none());
}

#[test]
fn databricks_completion_logs_the_opened_stream_once() {
    let mut fixture = Fixture::new();
    fixture.stream = Some(true);
    let handler = Recorder::succeeding();
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let outcome = complete_databricks(
        &fixture.context(DispatchGlobals::default()),
        &handler,
        &mut log,
    );
    assert!(outcome.is_ok());
    assert_eq!(
        state.model_call_details.get("original_response"),
        Some(&Json::Null)
    );
    assert_eq!(
        state.model_call_details.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
}

#[test]
fn snowflake_completion_passes_the_context_values_and_logs_only_a_failure() {
    let mut fixture = Fixture::new();
    fixture.provider = String::from("snowflake");
    fixture.model = String::from("snowflake/claude-3-5-sonnet");
    fixture.api_base = Some(String::from("https://acct.snowflakecomputing.com/api/v2"));
    fixture.api_key = Some(SecretString::from("pat/abc"));
    fixture.stream = Some(true);
    let redactor = redactor();
    let handler = Recorder::failing(400);
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let outcome = dispatch(
        ChatConfigTag::Snowflake,
        &fixture.context(DispatchGlobals::default()),
        &handler,
        &mut log,
    )
    .expect("the snowflake arm answers");
    assert!(matches!(
        outcome,
        Err(DispatchError::Handler(HandlerError::Upstream {
            status_code: 400
        }))
    ));
    let call = &handler.calls()[0];
    assert_eq!(call.custom_llm_provider, "snowflake");
    assert_eq!(
        call.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some(&String::from("pat/abc"))
    );
    assert!(state.model_call_details.get("original_response").is_some());

    let succeeding = Recorder::succeeding();
    let (mut second_state, second_env) = logging(&redactor);
    let mut second_log = CallLogging {
        state: &mut second_state,
        env: &second_env,
    };
    let outcome = dispatch(
        ChatConfigTag::Snowflake,
        &fixture.context(DispatchGlobals::default()),
        &succeeding,
        &mut second_log,
    )
    .expect("the snowflake arm answers");
    assert!(outcome.is_ok());
    assert!(
        second_state
            .model_call_details
            .get("original_response")
            .is_none()
    );
}

#[test]
fn oci_completion_passes_the_context_values_without_a_fallback() {
    let mut fixture = Fixture::new();
    fixture.provider = String::from("oci");
    fixture.model = String::from("oci/cohere.command-r-plus");
    let mut globals = OwnedGlobals::empty();
    globals.api_base = Some(String::from("https://global.example.com"));
    globals.api_key = Some(SecretString::from("global-key"));
    globals.headers = Some(BTreeMap::from([(
        String::from("X-Global"),
        String::from("1"),
    )]));
    let handler = Recorder::succeeding();
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let outcome = dispatch(
        ChatConfigTag::OciChat,
        &fixture.context(globals.borrowed()),
        &handler,
        &mut log,
    )
    .expect("the oci arm answers");
    assert!(outcome.is_ok());
    let call = &handler.calls()[0];
    assert_eq!(call.api_base, None);
    assert!(call.api_key.is_none());
    assert!(call.headers.is_none());
    assert_eq!(call.custom_llm_provider, "oci");
    assert!(state.model_call_details.get("original_response").is_none());
}

#[test]
fn dispatch_leaves_unknown_tags_to_the_later_arms() {
    let fixture = Fixture::new();
    let handler = Recorder::succeeding();
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let outcome = dispatch(
        ChatConfigTag::OpenAiGpt,
        &fixture.context(DispatchGlobals::default()),
        &handler,
        &mut log,
    );
    assert!(outcome.is_none());
    assert!(handler.calls().is_empty());
}

#[test]
fn embedding_config_selects_the_three_providers() {
    assert_eq!(
        embedding_config(ProviderId::Databricks),
        Some(EmbeddingConfig::Databricks)
    );
    assert_eq!(
        embedding_config(ProviderId::Snowflake),
        Some(EmbeddingConfig::Snowflake)
    );
    assert_eq!(
        embedding_config(ProviderId::Oci),
        Some(EmbeddingConfig::Oci)
    );
    assert_eq!(embedding_config(ProviderId::OpenAi), None);
}

#[test]
fn responses_selection_keeps_the_databricks_gpt_branch() {
    let facts = ResponsesModelFacts {
        azure: AzureResponsesFacts::default(),
        github_copilot_responses: false,
        mantle: MantleResponses::default(),
    };
    assert_eq!(
        select_responses_config(Some(ProviderId::Databricks), Some("databricks-gpt-5"), &facts),
        Some(betterlite_context::resource_config_selection::responses::ResponsesConfigTag::Databricks)
    );
    assert_eq!(
        select_responses_config(
            Some(ProviderId::Databricks),
            Some("databricks-llama-3"),
            &facts
        ),
        None
    );
    assert_eq!(
        select_responses_config(Some(ProviderId::Databricks), None, &facts),
        None
    );
}

#[tokio::test]
async fn databricks_embedding_sends_the_prepared_request() {
    let mut fixture = Fixture::new();
    fixture.model = String::from("databricks-bge-large-en");
    fixture.environment = environment(&[
        (
            "DATABRICKS_API_BASE",
            "https://dbc.example.com/serving-endpoints",
        ),
        ("DATABRICKS_API_KEY", "db-key"),
    ]);
    let transport = Scripted::with_replies(vec![ok(databricks_embedding_reply())]);
    let tokens = NoTokens;
    let clock = FixedClock;
    let call = fixture.embedding_call();
    let context = embedding_context(
        &fixture,
        DispatchGlobals::default(),
        &transport,
        &tokens,
        &clock,
    );
    let reply = embedding_dispatch(&call, EmbeddingConfig::Databricks, &context)
        .await
        .expect("the databricks embedding answers");
    let EmbeddingReply::Databricks(embedding) = reply else {
        panic!("expected a databricks embedding reply");
    };
    assert_eq!(embedding.model, "databricks-bge-large-en");
    let sent = transport.sent();
    assert_eq!(
        sent[0].url,
        "https://dbc.example.com/serving-endpoints/embeddings"
    );
    assert_eq!(
        sent[0]
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer db-key")
    );
    let body: PromptJson = serde_json::from_slice(&sent[0].body).expect("the body is JSON");
    let PromptJson::Object(fields) = body else {
        panic!("expected a JSON object body");
    };
    assert_eq!(
        fields.get("model"),
        Some(&PromptJson::Text(String::from("databricks-bge-large-en")))
    );
    assert_eq!(
        fields.get("input"),
        Some(&PromptJson::Text(String::from("hello")))
    );
}

#[tokio::test]
async fn databricks_embedding_reports_a_status_failure() {
    let mut fixture = Fixture::new();
    fixture.environment = environment(&[
        (
            "DATABRICKS_API_BASE",
            "https://dbc.example.com/serving-endpoints",
        ),
        ("DATABRICKS_API_KEY", "db-key"),
    ]);
    let transport = Scripted::with_replies(vec![status(500, "boom")]);
    let tokens = NoTokens;
    let clock = FixedClock;
    let call = fixture.embedding_call();
    let context = embedding_context(
        &fixture,
        DispatchGlobals::default(),
        &transport,
        &tokens,
        &clock,
    );
    match embedding_dispatch(&call, EmbeddingConfig::Databricks, &context).await {
        Err(DispatchError::Upstream { status: 500, body }) => assert_eq!(body, "boom"),
        other => panic!("expected the upstream status, got {other:?}"),
    }
}

#[tokio::test]
async fn snowflake_embedding_uses_the_cortex_url_and_snowflake_jwt() {
    let mut fixture = Fixture::new();
    fixture.model = String::from("openai-gpt-4.1");
    fixture.environment = environment(&[
        ("SNOWFLAKE_JWT", "pat/abc"),
        ("SNOWFLAKE_ACCOUNT_ID", "acct1"),
    ]);
    let transport = Scripted::with_replies(vec![ok(
        r#"{"object":"list","data":[{"object":"embedding","embedding":[[0.1,0.2]],"index":0}],"model":"openai-gpt-4.1","usage":{"prompt_tokens":1,"total_tokens":1}}"#,
    )]);
    let tokens = NoTokens;
    let clock = FixedClock;
    let call = fixture.embedding_call();
    let context = embedding_context(
        &fixture,
        DispatchGlobals::default(),
        &transport,
        &tokens,
        &clock,
    );
    let reply = embedding_dispatch(&call, EmbeddingConfig::Snowflake, &context)
        .await
        .expect("the snowflake embedding answers");
    let EmbeddingReply::Snowflake(embedding) = reply else {
        panic!("expected a snowflake embedding reply");
    };
    assert_eq!(embedding.hidden_model, "openai-gpt-4.1");
    assert_eq!(embedding.response.model, "snowflake/openai-gpt-4.1");
    let sent = transport.sent();
    assert_eq!(
        sent[0].url,
        "https://acct1.snowflakecomputing.com/api/v2/cortex/inference:embed"
    );
    assert_eq!(
        sent[0]
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer abc")
    );
    assert_eq!(
        sent[0]
            .headers
            .get("x-snowflake-authorization-token-type")
            .and_then(|value| value.to_str().ok()),
        Some("PROGRAMMATIC_ACCESS_TOKEN")
    );
    let body: PromptJson = serde_json::from_slice(&sent[0].body).expect("the body is JSON");
    let PromptJson::Object(fields) = body else {
        panic!("expected a JSON object body");
    };
    assert_eq!(
        fields.get("model"),
        Some(&PromptJson::Text(String::from("openai-gpt-4.1")))
    );
    assert_eq!(
        fields.get("text"),
        Some(&PromptJson::Text(String::from("hello")))
    );
}

#[tokio::test]
async fn snowflake_embedding_reports_the_upstream_status() {
    let mut fixture = Fixture::new();
    fixture.environment = environment(&[
        ("SNOWFLAKE_JWT", "pat/abc"),
        ("SNOWFLAKE_ACCOUNT_ID", "acct1"),
    ]);
    let transport = Scripted::with_replies(vec![status(400, "bad input")]);
    let tokens = NoTokens;
    let clock = FixedClock;
    let call = fixture.embedding_call();
    let context = embedding_context(
        &fixture,
        DispatchGlobals::default(),
        &transport,
        &tokens,
        &clock,
    );
    match embedding_dispatch(&call, EmbeddingConfig::Snowflake, &context).await {
        Err(DispatchError::SnowflakeException(exception)) => {
            assert_eq!(exception.status_code, 400);
            assert_eq!(exception.message, "bad input");
        }
        other => panic!("expected the snowflake exception, got {other:?}"),
    }
}

#[tokio::test]
async fn oci_embedding_signs_the_request_and_decodes_the_reply() {
    let mut fixture = Fixture::new();
    fixture.model = String::from("cohere.embed-english-v3.0");
    fixture.environment = environment(&[
        ("OCI_USER", "ocid1.user.oc1..bbbb"),
        ("OCI_FINGERPRINT", "aa:bb:cc"),
        ("OCI_TENANCY", "ocid1.tenancy.oc1..cccc"),
        ("OCI_KEY", TEST_KEY),
        ("OCI_COMPARTMENT_ID", "ocid1.compartment.oc1..aaaa"),
    ]);
    let transport = Scripted::with_replies(vec![ok(
        r#"{"embeddings":[[0.1,0.2]],"model_id":"cohere.embed-english-v3.0","model_version":"1.0"}"#,
    )]);
    let tokens = NoTokens;
    let clock = FixedClock;
    let call = fixture.embedding_call();
    let context = embedding_context(
        &fixture,
        DispatchGlobals::default(),
        &transport,
        &tokens,
        &clock,
    );
    let reply = embedding_dispatch(&call, EmbeddingConfig::Oci, &context)
        .await
        .expect("the oci embedding answers");
    let EmbeddingReply::Oci(embedding) = reply else {
        panic!("expected an oci embedding reply");
    };
    assert_eq!(embedding.model, "cohere.embed-english-v3.0");
    let sent = transport.sent();
    assert_eq!(
        sent[0].url,
        "https://inference.generativeai.us-ashburn-1.oci.oraclecloud.com/20231130/actions/embedText"
    );
    assert!(sent[0].headers.contains_key("authorization"));
    assert!(sent[0].headers.contains_key("x-content-sha256"));
    let body: PromptJson = serde_json::from_slice(&sent[0].body).expect("the body is JSON");
    let PromptJson::Object(fields) = body else {
        panic!("expected a JSON object body");
    };
    assert_eq!(
        fields.get("compartmentId"),
        Some(&PromptJson::Text(String::from(
            "ocid1.compartment.oc1..aaaa"
        )))
    );
    assert!(fields.contains_key("inputs"));
}

#[tokio::test]
async fn oauth_m2m_posts_the_form_and_reads_the_token() {
    let transport = Scripted::with_replies(vec![ok(r#"{"access_token":"tok"}"#)]);
    let request = oauth_token_request(
        "https://dbc.example.com/serving-endpoints",
        "client-id",
        "client-secret",
    );
    let token = request_oauth_token(
        &request,
        &OauthM2m {
            transport: &transport,
        },
    )
    .await
    .expect("the token reply is read");
    assert_eq!(token.expose_secret(), "tok");
    let sent = transport.sent();
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].url, "https://dbc.example.com/oidc/v1/token");
    assert_eq!(
        sent[0]
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        sent[0]
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Basic Y2xpZW50LWlkOmNsaWVudC1zZWNyZXQ=")
    );
    assert_eq!(
        sent[0].body.as_slice(),
        b"grant_type=client_credentials&scope=all-apis"
    );
    assert_eq!(sent[0].deadline, Duration::from_secs(30));
}

#[tokio::test]
async fn oauth_m2m_maps_an_error_status() {
    let transport = Scripted::with_replies(vec![status(401, "denied")]);
    let request = oauth_token_request("https://dbc.example.com", "id", "secret");
    match request_oauth_token(
        &request,
        &OauthM2m {
            transport: &transport,
        },
    )
    .await
    {
        Err(error) => {
            assert_eq!(error.status_code, 401);
            assert_eq!(error.message, "OAuth M2M token request failed: denied");
        }
        Ok(_) => panic!("expected the OAuth failure"),
    }
}

#[tokio::test]
async fn oauth_m2m_maps_a_transport_failure() {
    let transport = Scripted::default();
    let request = oauth_token_request("https://dbc.example.com", "id", "secret");
    match request_oauth_token(
        &request,
        &OauthM2m {
            transport: &transport,
        },
    )
    .await
    {
        Err(error) => {
            assert_eq!(error.status_code, 500);
            assert!(error.message.starts_with(
                "OAuth M2M token request failed: the provider answered with status 599"
            ));
        }
        Ok(_) => panic!("expected the OAuth failure"),
    }
}

#[test]
fn oauth_reply_decoding_keeps_the_status_and_the_body_text() {
    let error = parse_oauth_token_reply(500, "server error").expect_err("the status fails");
    assert_eq!(error.status_code, 500);
    assert_eq!(
        error.message,
        "OAuth M2M token request failed: server error"
    );
    let token =
        parse_oauth_token_reply(200, r#"{"access_token":"abc"}"#).expect("the token reply is read");
    assert_eq!(token.expose_secret(), "abc");
}

fn sse_head(events: Vec<&'static str>) -> StreamHead {
    let chunks = events
        .into_iter()
        .map(|event| Ok(event.as_bytes().to_vec()))
        .collect::<Vec<Result<Vec<u8>, HttpError>>>();
    StreamHead {
        status: 200,
        body: Vec::new(),
        chunks: Some(stream::iter(chunks).boxed()),
    }
}

#[tokio::test]
async fn oci_stream_sends_the_signed_body_and_decodes_the_events() {
    let transport = Scripted::with_stream(sse_head(vec![
        "data: {\"message\":{\"content\":[{\"type\":\"TEXT\",\"text\":\"he\"}]}}\n",
        "\ndata: {\"message\":{\"content\":[{\"type\":\"TEXT\",\"text\":\"llo\"}]}}\n\ndata: [DONE]\n",
    ]));
    let headers = HeaderMap::new();
    let data = PromptJson::Object(Object::from_iter([(
        String::from("model"),
        PromptJson::Text(String::from("cohere.command-r-plus")),
    )]));
    let mut stream = open_stream(
        OciStreamRequest {
            api_base: "https://inference.generativeai.us-ashburn-1.oci.oraclecloud.com/20231130/actions/chat",
            headers,
            data: &data,
            signed_body: Some(b"{\"signed\":true}".to_vec()),
        },
        &transport,
    )
    .await
    .expect("the stream opens");
    let mut contents = Vec::new();
    let mut saw_done = false;
    while let Some(item) = stream.next().await.expect("the stream decodes") {
        match item {
            StreamItem::Done => {
                saw_done = true;
                break;
            }
            StreamItem::Chunk(chunk) => {
                if let Some(content) = chunk.choice.delta.content {
                    contents.push(content);
                }
            }
        }
    }
    assert_eq!(contents, vec![String::from("he"), String::from("llo")]);
    assert!(saw_done);
    assert!(stream.next().await.expect("the stream ends").is_none());
    let sent = transport.sent();
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].body.as_slice(), b"{\"signed\":true}");
    assert_eq!(sent[0].deadline, Duration::from_secs(300));
}

#[tokio::test]
async fn oci_stream_sends_the_json_body_without_a_signature() {
    let transport = Scripted::with_stream(sse_head(vec!["data: [DONE]\n"]));
    let headers = HeaderMap::new();
    let data = PromptJson::Object(Object::from_iter([(
        String::from("model"),
        PromptJson::Text(String::from("cohere.command-r-plus")),
    )]));
    let stream = open_stream(
        OciStreamRequest {
            api_base: "https://inference.generativeai.us-ashburn-1.oci.oraclecloud.com/20231130/actions/chat",
            headers,
            data: &data,
            signed_body: None,
        },
        &transport,
    )
    .await;
    assert!(stream.is_ok());
    let sent = transport.sent();
    let body: PromptJson = serde_json::from_slice(&sent[0].body).expect("the body is JSON");
    assert_eq!(
        body,
        PromptJson::Object(Object::from_iter([(
            String::from("model"),
            PromptJson::Text(String::from("cohere.command-r-plus")),
        )]))
    );
}

#[tokio::test]
async fn oci_stream_maps_a_status_failure_to_the_oci_error() {
    let transport = Scripted::with_stream(StreamHead {
        status: 500,
        body: b"boom".to_vec(),
        chunks: None,
    });
    let headers = HeaderMap::new();
    let data = PromptJson::Null;
    match open_stream(
        OciStreamRequest {
            api_base: "https://inference.generativeai.us-ashburn-1.oci.oraclecloud.com/20231130/actions/chat",
            headers,
            data: &data,
            signed_body: None,
        },
        &transport,
    )
    .await
    {
        Err(DispatchError::Oci(error)) => {
            assert_eq!(error.status, 500);
            assert_eq!(error.message, "boom");
        }
        other => panic!("expected the OCI status error, got {other:?}"),
    }
}

#[tokio::test]
async fn oci_stream_carries_a_utf8_sequence_split_across_reads() {
    let line = "data: {\"message\":{\"content\":[{\"type\":\"TEXT\",\"text\":\"héllo\"}]}}\n\n";
    let bytes = line.as_bytes();
    let split = line
        .find('é')
        .expect("the fixture holds a two-byte sequence")
        + 1;
    let first = Ok(bytes[..split].to_vec());
    let second = Ok(bytes[split..].to_vec());
    let transport = Scripted::with_stream(StreamHead {
        status: 200,
        body: Vec::new(),
        chunks: Some(stream::iter(vec![first, second]).boxed()),
    });
    let headers = HeaderMap::new();
    let data = PromptJson::Null;
    let mut stream = open_stream(
        OciStreamRequest {
            api_base: "https://example.com",
            headers,
            data: &data,
            signed_body: None,
        },
        &transport,
    )
    .await
    .expect("the stream opens");
    let mut contents = Vec::new();
    while let Some(item) = stream.next().await.expect("the stream decodes") {
        if let StreamItem::Chunk(chunk) = item
            && let Some(content) = chunk.choice.delta.content
        {
            contents.push(content);
        }
    }
    assert_eq!(contents, vec![String::from("héllo")]);
}

#[tokio::test]
async fn snowflake_reply_transform_keeps_the_source_model() {
    let reply = transform_embedding_response(
        "openai-gpt-4.1",
        r#"{"object":"list","data":[{"object":"embedding","embedding":[[0.1]],"index":0}],"model":"openai-gpt-4.1","usage":{"prompt_tokens":1,"total_tokens":1}}"#,
    )
    .expect("the snowflake reply is read");
    assert_eq!(reply.hidden_model, "openai-gpt-4.1");
}
