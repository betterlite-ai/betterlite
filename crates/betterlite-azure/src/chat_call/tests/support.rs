#![cfg_attr(
    test,
    expect(
        clippy::disallowed_types,
        clippy::too_many_lines,
        reason = "B505's raw chat transports and the scripted replies they return speak serde_json values, and one harness assembles a whole completion call"
    )
)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::azure_endpoints::Settings as AzureSettings;
use betterlite_config::sections::openai_chat_content::Settings as ChatContentSettings;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::PostCall;
use betterlite_models::decisions;
use betterlite_openai::output_limits::raw_send::{
    AsyncRawChatTransport, BadRequestReply, RawChatError, RawChatResponse, StatusReply,
    SyncRawChatTransport,
};
use betterlite_openai::output_limits::tokens::{PromptCountError, PromptTokens};
use betterlite_openai::text_completion_transport::logging::{SecretHeaders, StreamDefaults};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::JsonObject;
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value, json};
use tokio::runtime::Builder;
use uuid::Uuid;

use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::azure_stream_errors::records::{GetStreamClient, PreCallRecord, StreamClient};
use crate::chat_call::CancellationToken;
use crate::chat_call::arguments::{ChatCall, Runtime};
use crate::chat_call::client::{ClientHandle, ClientSource, GatewayClient, GetClient};
use crate::chat_call::error::AzureChatError;
use crate::chat_call::handler::{self, ChatOutcome};
use crate::chat_call::hooks::ChatHooks;
use crate::credentials::source::{CallableTokenProvider, CredentialSources, SourceFailure};
use crate::endpoints::cloudflare::CloudflareParams;
use crate::endpoints::initialize::EndpointError;
use crate::endpoints::projection::{ClientKind, LiteLlmParams};
use crate::oidc_exchange::OidcExchangeInput;

pub(super) const CLOCK: i64 = 1_700_000_000;
pub(super) const TOKENS: usize = 11;

pub(super) const DEFAULT_BODY: &str = concat!(
    r#"{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"#,
    r#""model":"gpt-4o","choices":[{"index":0,"#,
    r#""message":{"role":"assistant","content":"Hello!"},"finish_reason":"stop"}],"#,
    r#""usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#
);

pub(super) const fn identifier() -> Uuid {
    Uuid::from_u128(7)
}

pub(super) const fn clock() -> i64 {
    CLOCK
}

pub(super) const fn recovered_clock() -> u64 {
    1_700_000_000_u64
}

pub(super) const fn now() -> Duration {
    Duration::from_secs(1)
}

pub(super) const fn stream_instant() -> StreamInstant {
    StreamInstant {
        seconds: CLOCK,
        timestamp: 1_700_000_000.0,
    }
}

// The answer the scripted transport hands B505's send wrappers
#[derive(Clone)]
pub(super) enum Answer {
    Reply {
        headers: Vec<(String, String)>,
        body: String,
    },
    Status {
        status: u16,
        message: String,
        headers: Vec<(String, String)>,
        body: String,
    },
    BadRequest {
        message: String,
        headers: Vec<(String, String)>,
        body: String,
    },
    Timeout {
        message: String,
    },
    Other {
        message: String,
    },
    Pending,
}

impl Answer {
    pub(super) fn reply() -> Self {
        Self::Reply {
            headers: json_headers(),
            body: DEFAULT_BODY.to_owned(),
        }
    }

    pub(super) fn with_body(body: &str) -> Self {
        Self::Reply {
            headers: json_headers(),
            body: body.to_owned(),
        }
    }

    pub(super) fn status(status: u16, message: &str, body: &str) -> Self {
        Self::Status {
            status,
            message: message.to_owned(),
            headers: json_headers(),
            body: body.to_owned(),
        }
    }

    fn make(&self) -> Result<RawChatResponse, RawChatError> {
        match self {
            Self::Reply { headers, body } => Ok(RawChatResponse {
                headers: header_map(headers),
                body: body_value(body),
            }),
            Self::Status {
                status,
                message,
                headers,
                body,
            } => Err(RawChatError::Status(Box::new(StatusReply {
                status: *status,
                message: message.clone(),
                headers: header_map(headers),
                body: body_value(body),
            }))),
            Self::BadRequest {
                message,
                headers,
                body,
            } => Err(RawChatError::BadRequest(Box::new(BadRequestReply {
                message: message.clone(),
                headers: header_map(headers),
                body: body_value(body),
            }))),
            Self::Timeout { message } => Err(RawChatError::Timeout {
                message: message.clone(),
            }),
            Self::Other { message } => Err(RawChatError::Other {
                message: message.clone(),
            }),
            Self::Pending => Err(RawChatError::Other {
                message: "the scripted answer never arrives on the sync transport".to_owned(),
            }),
        }
    }
}

pub(super) fn json_headers() -> Vec<(String, String)> {
    vec![("content-type".to_owned(), "application/json".to_owned())]
}

fn body_value(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_source| Value::String(text.to_owned()))
}

fn header_map(headers: &[(String, String)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (raw_name, raw_value) in headers {
        let name = HeaderName::from_bytes(raw_name.as_bytes()).expect("fixture header name");
        let value = HeaderValue::from_str(raw_value).expect("fixture header value");
        map.insert(name, value);
    }
    map
}

#[derive(Clone)]
pub(super) struct RequestRecord {
    pub data: Value,
    pub timeout: f64,
}

#[derive(Clone)]
pub(super) struct GetRecord {
    pub api_version: Option<String>,
    pub api_base: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub is_async: bool,
    pub litellm_params: LiteLlmParams,
    pub supplied: bool,
}

#[derive(Clone)]
pub(super) struct GatewayRecord {
    pub api_base: String,
    pub model: String,
    pub api_version: Option<String>,
    pub max_retries: Value,
    pub timeout: f64,
    pub api_key: Option<String>,
    pub acompletion: bool,
}

#[derive(Clone)]
pub(super) struct ScriptedTransport {
    answer: Answer,
    requests: Arc<Mutex<Vec<RequestRecord>>>,
}

impl ScriptedTransport {
    fn record(&self, data: &Map<String, Value>, timeout: f64) {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RequestRecord {
                data: Value::Object(data.clone()),
                timeout,
            });
    }
}

impl SyncRawChatTransport for ScriptedTransport {
    fn create(
        &self,
        data: &Map<String, Value>,
        timeout: f64,
    ) -> Result<RawChatResponse, RawChatError> {
        self.record(data, timeout);
        self.answer.make()
    }
}

#[async_trait]
impl AsyncRawChatTransport for ScriptedTransport {
    async fn create(
        &self,
        data: &Map<String, Value>,
        timeout: f64,
    ) -> Result<RawChatResponse, RawChatError> {
        if matches!(self.answer, Answer::Pending) {
            self.record(data, timeout);
            return std::future::pending::<Result<RawChatResponse, RawChatError>>().await;
        }
        SyncRawChatTransport::create(self, data, timeout)
    }
}

// The client get_azure_openai_client hands back, plus what the call sites asked it for
pub(super) struct ScriptedSource {
    kind: ClientKind,
    api_key: Option<SecretString>,
    base_url: String,
    answer: Answer,
    pub requests: Arc<Mutex<Vec<RequestRecord>>>,
    pub gets: Arc<Mutex<Vec<GetRecord>>>,
    pub gateways: Arc<Mutex<Vec<GatewayRecord>>>,
}

impl ScriptedSource {
    pub(super) fn new(kind: ClientKind, api_key: Option<&str>, answer: Answer) -> Self {
        Self {
            kind,
            api_key: api_key.map(SecretString::from),
            base_url: "https://unit.test/openai/".to_owned(),
            answer,
            requests: Arc::new(Mutex::new(Vec::new())),
            gets: Arc::new(Mutex::new(Vec::new())),
            gateways: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn handle(&self) -> ClientHandle<ScriptedTransport> {
        ClientHandle {
            kind: self.kind,
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            transport: ScriptedTransport {
                answer: self.answer.clone(),
                requests: Arc::clone(&self.requests),
            },
        }
    }

    fn record<Record>(
        &self,
        slot: &Arc<Mutex<Vec<Record>>>,
        value: Record,
    ) -> Result<ClientHandle<ScriptedTransport>, EndpointError> {
        slot.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(value);
        Ok(self.handle())
    }

    pub(super) fn requests(&self) -> Vec<RequestRecord> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn gets(&self) -> Vec<GetRecord> {
        self.gets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn gateways(&self) -> Vec<GatewayRecord> {
        self.gateways
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for ScriptedSource {
    type Client = ScriptedTransport;

    fn get(&self, call: GetClient<'_>) -> Result<ClientHandle<Self::Client>, EndpointError> {
        self.record(
            &self.gets,
            GetRecord {
                api_version: call.api_version.map(str::to_owned),
                api_base: call.api_base.map(str::to_owned),
                api_key: call.api_key.map(|key| key.expose_secret().to_owned()),
                model: call.model.map(str::to_owned),
                is_async: call.is_async,
                litellm_params: call.litellm_params.clone(),
                supplied: call.supplied.is_some(),
            },
        )
    }

    fn gateway(
        &self,
        call: GatewayClient<'_>,
    ) -> Result<ClientHandle<Self::Client>, EndpointError> {
        self.record(
            &self.gateways,
            GatewayRecord {
                api_base: call.api_base.to_owned(),
                model: call.model.to_owned(),
                api_version: call.api_version.map(str::to_owned),
                max_retries: serde_json::to_value(&call.max_retries.0).unwrap_or(Value::Null),
                timeout: call.timeout,
                api_key: call.api_key.map(|key| key.expose_secret().to_owned()),
                acompletion: call.acompletion,
            },
        )
    }

    // the owned cases never stream; B685 owns the streaming path
    fn stream(
        &self,
        _supplied: Option<&ClientHandle<Self::Client>>,
        _call: GetStreamClient<'_>,
    ) -> Result<StreamClient, EndpointError> {
        Err(EndpointError::MissingApiVersion)
    }
}

// The logging object the handler calls: the tests record the pre and post calls instead of emitting them
#[derive(Default)]
pub(super) struct Recorder {
    pub pre: Vec<PreCallRecord>,
    pub post: Vec<PostCall>,
    pub response_headers: Vec<IndexMap<String, String>>,
}

impl ChatHooks for Recorder {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        self.pre.push(record);
        Ok(())
    }

    fn post_call(&mut self, call: PostCall) {
        self.post.push(call);
    }

    fn response_headers(&mut self, headers: &IndexMap<String, String>) {
        self.response_headers.push(headers.clone());
    }

    fn stream_defaults(&self) -> StreamDefaults {
        StreamDefaults {
            stream_options: None,
            merge_reasoning_in_choices: false,
            is_function_call: false,
            base_hidden_params: JsonObject::new(),
        }
    }
}

pub(super) struct FixedTokens;

#[async_trait]
impl PromptTokens for FixedTokens {
    async fn count(&self, _model: &str, _messages: &[Value]) -> Result<usize, PromptCountError> {
        Ok(TOKENS)
    }
}

pub(super) struct NoCredentials;

fn unscripted() -> SourceFailure {
    SourceFailure {
        class: "KeyError".to_owned(),
        status: None,
        message: "the owned chat path never resolves a credential".to_owned(),
    }
}

impl CredentialSources for NoCredentials {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unscripted())
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unscripted())
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unscripted())
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unscripted())
    }
}

// One call's arguments, kept owned so the borrows a ChatCall needs live for its whole run
pub(super) struct Harness {
    pub model: Option<String>,
    pub messages: Option<Vec<Message>>,
    pub api_key: Option<String>,
    pub azure_ad_token: Option<String>,
    pub api_base: String,
    pub api_version: String,
    pub timeout: f64,
    pub optional_params: Value,
    pub litellm_params: LiteLlmParams,
    pub base_model: Option<String>,
    pub acompletion: bool,
    pub headers: Option<BTreeMap<String, String>>,
    pub default_max_retries: i64,
    pub system_messages_first: bool,
    pub cancel: bool,
    pub supplied: bool,
}

impl Default for Harness {
    fn default() -> Self {
        Self {
            model: Some("gpt-4o-deployment".to_owned()),
            messages: Some(messages_of(json!([{"role": "user", "content": "hi"}]))),
            api_key: Some("key".to_owned()),
            azure_ad_token: None,
            api_base: "https://unit.test".to_owned(),
            api_version: "2024-02-01".to_owned(),
            timeout: 30.0,
            optional_params: json!({}),
            litellm_params: LiteLlmParams::default(),
            base_model: None,
            acompletion: false,
            headers: None,
            default_max_retries: 2,
            system_messages_first: false,
            cancel: false,
            supplied: true,
        }
    }
}

impl Harness {
    pub(super) async fn run(
        &self,
        source: &ScriptedSource,
        hooks: &mut Recorder,
    ) -> Result<ChatOutcome, AzureChatError> {
        let api_key = self.api_key.clone().map(SecretString::from);
        let token = self.azure_ad_token.clone().map(SecretString::from);
        let headers = self.headers.clone().map(SecretHeaders);
        let mut params = object_of(self.optional_params.clone());
        let litellm_params = self.litellm_params.clone();
        let azure = AzureSettings {
            default_max_retries: self.default_max_retries,
            azure_default_api_version: self.api_version.clone(),
        };
        let chat = ChatContentSettings {
            openai_system_messages_first: self.system_messages_first,
        };
        let sources = NoCredentials;
        let cancel = self.cancel.then(CancellationToken::new);
        if let Some(token_to_cancel) = &cancel {
            token_to_cancel.cancel();
        }
        let tokens = FixedTokens;
        let runtime = Runtime {
            identifier,
            clock,
            recovered_clock,
            now,
            stream_now: stream_instant,
            prompt_tokens: &tokens,
        };
        let supplied = self.supplied.then(|| source.handle());
        let mut call = ChatCall {
            model: self.model.as_deref(),
            messages: self.messages.as_deref(),
            api_key: api_key.as_ref(),
            api_base: &self.api_base,
            api_version: &self.api_version,
            azure_ad_token: token.as_ref(),
            azure_ad_token_provider: None,
            dynamic_params: false,
            timeout: self.timeout,
            optional_params: &mut params,
            litellm_params,
            base_model: self.base_model.as_deref(),
            cloudflare_params: CloudflareParams::default(),
            acompletion: self.acompletion,
            headers: headers.as_ref(),
            supplied,
            sources: &sources,
            azure: &azure,
            chat: &chat,
            cancel: cancel.as_ref(),
        };
        handler::completion(source, &mut call, &runtime, hooks).await
    }
}

pub(super) fn block_on<Fut>(future: Fut) -> Fut::Output
where
    Fut: Future,
{
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    runtime.block_on(future)
}

pub(super) fn object_of(value: Value) -> Object {
    serde_json::from_value(value).expect("an object")
}

pub(super) fn messages_of(value: Value) -> Vec<Message> {
    serde_json::from_value(value).expect("messages")
}

pub(super) fn decisions_of(value: Value) -> decisions::Json {
    serde_json::from_value(value).expect("a decisions value")
}

pub(super) fn status_of(error: &AzureChatError) -> Option<u16> {
    match error {
        AzureChatError::Azure { status, .. } => Some(*status),
        AzureChatError::Cancelled => None,
    }
}
