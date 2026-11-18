#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, UNIX_EPOCH};

use async_trait::async_trait;
use aws_smithy_async::time::{SharedTimeSource, StaticTimeSource};
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::sagemaker_runtime::runtime::{ChunkStream, SagemakerEnvironment};
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::embedding::EmbeddingVector;
use betterlite_openai::sagemaker_completion::error::SagemakerError;
use betterlite_openai::sagemaker_completion::request::CustomPromptEntry;
use betterlite_openai::sagemaker_completion::response::TextTokenCounter;
use betterlite_openai::watsonx_auth::error::WatsonxError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::Hub;
use betterlite_prompts::message_content::Message;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_tokenizer::engine::error::EngineError;
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use crate::data_cloud_dispatch::call::{CompletionCall, EmbeddingCall};
use crate::data_cloud_dispatch::context::{DispatchContext, DispatchGlobals, EnvironmentSource};
use crate::data_cloud_dispatch::handler::{ChatCompletionHandler, HandlerError};
use crate::sap_auth::error::SapAuthError;
use crate::sap_auth::token::SapClock;
use crate::sap_chat::error::SapChatError;
use crate::sap_embedding::error::SapEmbeddingError;
use crate::sap_stream::SapOrchestrationError;

use super::call::SagemakerRequest;
use super::chat::{self, Completion, SAP_MODEL_VERSION};
use super::context::ChatContext;
use super::embed::{self, EmbeddingContext, EmbeddingReply};
use super::error::EnterpriseError;
use super::sagemaker::{SagemakerDeps, SagemakerReply, Tokenizer};
use super::sap::{self, SapRequest};
use super::select::{EmbeddingConfig, RerankConfig, embedding_config, rerank_config};
use super::transport::{AwsCredentials, EnterpriseTransport};
use super::watsonx;

const NO_HOME: &str = "/nonexistent-b8331-home";

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
    fn now(&self) -> Instant {
        Instant::now()
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

fn key(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn object(wire: &str) -> Object {
    serde_json::from_str(wire).expect("the fixture object is JSON")
}

fn message(wire: &str) -> Message {
    serde_json::from_str(wire).expect("the fixture message is a chat message")
}

#[derive(Clone)]
struct Sent {
    method: String,
    url: String,
    headers: http::HeaderMap,
    body: Vec<u8>,
    deadline: Duration,
}

fn response(status: u16, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: http::HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

struct Scripted {
    replies: Mutex<VecDeque<UpstreamResponse>>,
    sent: Mutex<Vec<Sent>>,
    streamed: Mutex<bool>,
}

impl Scripted {
    fn new(replies: Vec<(u16, &str)>) -> Self {
        Self {
            replies: Mutex::new(
                replies
                    .into_iter()
                    .map(|(code, body)| response(code, body))
                    .collect(),
            ),
            sent: Mutex::new(Vec::new()),
            streamed: Mutex::new(false),
        }
    }

    fn empty() -> Self {
        Self::new(Vec::new())
    }

    fn sent(&self) -> Vec<Sent> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn streamed(&self) -> bool {
        *self.streamed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn body(&self, index: usize) -> serde_json::Value {
        let sent = self.sent();
        serde_json::from_slice(&sent[index].body).expect("the fixture recorded JSON")
    }

    fn header(&self, index: usize, name: http::header::HeaderName) -> Option<String> {
        let sent = self.sent();
        sent[index]
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    }

    fn record(&self, request: &OutboundRequest) {
        let body = match &request.body {
            RequestBody::Json(bytes) => bytes.clone(),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Vec::new()
            }
        };
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Sent {
                method: request.method.to_string(),
                url: request.url.to_string(),
                headers: request.headers.clone(),
                body,
                deadline: request.deadline,
            });
    }
}

#[async_trait]
impl EnterpriseTransport for Scripted {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        self.record(&request);
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or(TransportError::Closed)
    }

    async fn open_stream(
        &self,
        request: OutboundRequest,
    ) -> Result<(u16, ChunkStream), TransportError> {
        self.record(&request);
        *self.streamed.lock().unwrap_or_else(PoisonError::into_inner) = true;
        Err(TransportError::Closed)
    }
}

struct StubCredentials;

#[async_trait]
impl AwsCredentials for StubCredentials {
    async fn resolve(
        &self,
        _settings: AwsSettings,
        _region: String,
    ) -> Result<Credentials, CredentialError> {
        Ok(Credentials {
            access_key_id: key("AKIAFIXTURE"),
            secret_access_key: key("fixture-secret"),
            session_token: None,
        })
    }
}

struct Counter;

impl TextTokenCounter for Counter {
    fn count_response_tokens(&self, text: &str) -> u64 {
        u64::try_from(text.split_whitespace().count()).unwrap_or_default()
    }
}

struct NeverEncoding;

impl TextEncoding for NeverEncoding {
    fn count_tokens(&self, _text: &str) -> Result<usize, EngineError> {
        Err(EngineError::PoolClosed)
    }
}

fn identifier() -> Uuid {
    Uuid::from_u128(7)
}

fn prompt_factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(BTreeMap::new()),
        hub: Hub::Unreachable,
    }
}

struct Sagemaker {
    transport: Scripted,
    credentials: StubCredentials,
    counter: Counter,
    encoding: NeverEncoding,
    prompts: PromptFactory,
}

impl Sagemaker {
    fn new(transport: Scripted) -> Self {
        Self {
            transport,
            credentials: StubCredentials,
            counter: Counter,
            encoding: NeverEncoding,
            prompts: prompt_factory(),
        }
    }

    fn deps(&self) -> SagemakerDeps<'_> {
        SagemakerDeps {
            transport: &self.transport,
            credentials: &self.credentials,
            environment: SagemakerEnvironment {
                region_name: Some(String::from("us-west-2")),
                region: None,
                session_region: None,
            },
            clock: SharedTimeSource::new(StaticTimeSource::new(UNIX_EPOCH)),
            counter: &self.counter,
            encoding: &self.encoding,
            prompts: &self.prompts,
            timeout: Duration::from_secs(5),
            identifier,
        }
    }
}

struct Chat {
    provider: &'static str,
    model: String,
    api_base: Option<String>,
    api_key: Option<SecretString>,
    headers: Option<BTreeMap<String, String>>,
    optional_params: Object,
    messages: Vec<Message>,
    litellm_params: Object,
    globals_headers: Option<BTreeMap<String, String>>,
    provider_config: Option<ChatConfigTag>,
    environment: BTreeMap<String, String>,
    secrets: SecretLookup,
    sagemaker_prompts: Option<BTreeMap<String, CustomPromptEntry>>,
}

impl Chat {
    fn new(provider: &'static str, model: &str) -> Self {
        Self {
            provider,
            model: model.to_owned(),
            api_base: None,
            api_key: None,
            headers: None,
            optional_params: Object::new(),
            messages: vec![message(r#"{"role":"user","content":"hi"}"#)],
            litellm_params: Object::new(),
            globals_headers: None,
            provider_config: None,
            environment: BTreeMap::new(),
            secrets: secrets(),
            sagemaker_prompts: None,
        }
    }

    fn context(&self) -> ChatContext<'_> {
        ChatContext {
            base: DispatchContext {
                acompletion: false,
                api_base: self.api_base.as_deref(),
                api_key: self.api_key.as_ref(),
                custom_llm_provider: self.provider,
                headers: self.headers.as_ref(),
                litellm_params: &self.litellm_params,
                messages: &self.messages,
                model: &self.model,
                optional_params: &self.optional_params,
                provider_config: self.provider_config,
                stream: None,
                timeout: Some(Duration::from_secs(600)),
                globals: DispatchGlobals {
                    api_base: None,
                    api_key: None,
                    databricks_key: None,
                    headers: self.globals_headers.as_ref(),
                },
                env: EnvironmentSource {
                    secrets: &self.secrets,
                    environment: &self.environment,
                },
            },
            sagemaker: SagemakerRequest {
                custom_prompt_dict: self.sagemaker_prompts.as_ref(),
                global_custom_prompt_dict: None,
                hf_model_name: None,
            },
        }
    }
}

struct Recorder {
    calls: Mutex<Vec<CompletionCall>>,
    reply: String,
    failure: Option<u16>,
}

impl Recorder {
    fn answer(reply: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            reply: reply.to_owned(),
            failure: None,
        }
    }

    fn failing(status_code: u16) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            reply: String::new(),
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

    fn complete(&self, call: &CompletionCall) -> Result<String, HandlerError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.clone());
        self.failure.map_or_else(
            || Ok(self.reply.clone()),
            |status_code| Err(HandlerError::Upstream { status_code }),
        )
    }
}

fn exposed(value: Option<&SecretString>) -> Option<String> {
    value.map(|stored| stored.expose_secret().to_owned())
}

struct FixedClock;

impl SapClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("the fixture time converts")
    }
}

fn sap_environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    let mut environment: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    environment.insert(String::from("AICORE_HOME"), String::from(NO_HOME));
    environment
}

fn embedding_call(model: &str, api_key: Option<SecretString>) -> EmbeddingCall {
    EmbeddingCall {
        model: model.to_owned(),
        input: Json::Text(String::from("hi")),
        optional_params: Object::new(),
        litellm_params: Object::new(),
        api_key,
        api_base: None,
        headers: None,
        timeout: Duration::from_secs(5),
    }
}

const SAP_SERVICE_KEY: &str = r#"{"client_id":"ignored","client_secret":"ignored","url":"https://other-auth.example.com","serviceurls":{"AI_API_URL":"https://other-base.example.com"}}"#;

fn sap_credentials_environment() -> BTreeMap<String, String> {
    sap_environment(&[
        ("AICORE_CLIENT_ID", "client"),
        ("AICORE_CLIENT_SECRET", "secret"),
        ("AICORE_AUTH_URL", "https://auth.example.com"),
        ("AICORE_BASE_URL", "https://base.example.com"),
    ])
}

#[test]
fn dispatch_leaves_the_other_providers_alone() {
    let chat = Chat::new("openai", "gpt-4o");
    let recorder = Recorder::answer("unused");
    let sagemaker = Sagemaker::new(Scripted::empty());
    for tag in [ChatConfigTag::OpenAiGpt, ChatConfigTag::OpenAiGpt5] {
        let outcome = chat::dispatch::<String>(tag, &chat.context(), &sagemaker.deps(), &recorder);
        assert!(outcome.is_none());
    }
    assert!(recorder.calls().is_empty());
}

#[test]
fn sap_chat_merges_the_default_model_version() {
    let mut chat = Chat::new("sap", "gpt-4o");
    chat.provider_config = Some(ChatConfigTag::GenAiHubOrchestration);
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    let outcome = chat::dispatch::<String>(
        ChatConfigTag::GenAiHubOrchestration,
        &chat.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the sap arm runs")
    .expect("the handler answers");
    assert!(matches!(outcome, Completion::Handler(reply) if reply == "reply"));
    let sent = recorder.calls();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].optional_params,
        object(&format!(r#"{{"model_version":"{SAP_MODEL_VERSION}"}}"#))
    );
    assert_eq!(sent[0].custom_llm_provider, "sap");
    assert!(chat.optional_params.is_empty());
}

#[test]
fn sap_chat_keeps_a_model_version_the_caller_set() {
    let mut chat = Chat::new("sap", "gpt-4o");
    chat.optional_params = object(r#"{"model_version":"2405"}"#);
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    chat::dispatch::<String>(
        ChatConfigTag::GenAiHubOrchestration,
        &chat.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the sap arm runs")
    .expect("the handler answers");
    assert_eq!(
        recorder.calls()[0].optional_params,
        object(r#"{"model_version":"2405"}"#)
    );

    let mut null_version = Chat::new("sap", "gpt-4o");
    null_version.optional_params = object(r#"{"model_version":null}"#);
    let recorder = Recorder::answer("reply");
    chat::dispatch::<String>(
        ChatConfigTag::GenAiHubOrchestration,
        &null_version.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the sap arm runs")
    .expect("the handler answers");
    assert_eq!(
        recorder.calls()[0].optional_params,
        object(r#"{"model_version":null}"#)
    );
}

#[test]
fn sap_chat_falls_back_to_the_global_headers() {
    let mut chat = Chat::new("sap", "gpt-4o");
    chat.headers = Some(BTreeMap::new());
    chat.globals_headers = Some(BTreeMap::from([(
        String::from("x-global"),
        String::from("1"),
    )]));
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    chat::dispatch::<String>(
        ChatConfigTag::GenAiHubOrchestration,
        &chat.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the sap arm runs")
    .expect("the handler answers");
    assert_eq!(
        recorder.calls()[0].headers,
        Some(BTreeMap::from([(
            String::from("x-global"),
            String::from("1")
        )]))
    );

    let mut explicit = Chat::new("sap", "gpt-4o");
    explicit.headers = Some(BTreeMap::from([(
        String::from("x-explicit"),
        String::from("2"),
    )]));
    explicit.globals_headers = Some(BTreeMap::from([(
        String::from("x-global"),
        String::from("1"),
    )]));
    let recorder = Recorder::answer("reply");
    chat::dispatch::<String>(
        ChatConfigTag::GenAiHubOrchestration,
        &explicit.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the sap arm runs")
    .expect("the handler answers");
    assert_eq!(
        recorder.calls()[0].headers,
        Some(BTreeMap::from([(
            String::from("x-explicit"),
            String::from("2")
        )]))
    );
}

#[test]
fn sap_chat_reports_the_handler_failure() {
    let mut chat = Chat::new("sap", "gpt-4o");
    chat.provider_config = Some(ChatConfigTag::GenAiHubOrchestration);
    let recorder = Recorder::failing(502);
    let sagemaker = Sagemaker::new(Scripted::empty());
    let error = chat::dispatch::<String>(
        ChatConfigTag::GenAiHubOrchestration,
        &chat.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the sap arm runs")
    .expect_err("the handler fails");
    assert!(matches!(error, EnterpriseError::Handler(_)));
    assert_eq!(error.status(), Some(502));
}

#[test]
fn watsonx_chat_passes_the_context_fields_under_the_literal_provider() {
    let mut chat = Chat::new("watsonx_chat", "ibm/granite");
    chat.api_base = Some(String::from("https://wx.example.com"));
    chat.api_key = Some(key("ctx-key"));
    chat.optional_params = object(r#"{"temperature":0.2}"#);
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    chat::dispatch::<String>(
        ChatConfigTag::IbmWatsonXChat,
        &chat.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the watsonx arm runs")
    .expect("the handler answers");
    let sent = recorder.calls();
    assert_eq!(sent[0].custom_llm_provider, "watsonx");
    assert_eq!(sent[0].api_base.as_deref(), Some("https://wx.example.com"));
    assert_eq!(
        exposed(sent[0].api_key.as_ref()).as_deref(),
        Some("ctx-key")
    );
    assert_eq!(sent[0].optional_params, object(r#"{"temperature":0.2}"#));
}

#[test]
fn watsonx_text_keeps_the_parameter_keys_the_context_overrides() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.api_base = Some(String::from("https://ctx.example.com"));
    chat.api_key = Some(key("ctx-key"));
    chat.optional_params = object(
        r#"{"apikey":"param-key","url":"https://url.example.com","base_url":"https://legacy.example.com"}"#,
    );
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    chat::dispatch::<String>(
        ChatConfigTag::IbmWatsonXAi,
        &chat.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the text arm runs")
    .expect("the handler answers");
    let sent = recorder.calls();
    assert_eq!(sent[0].custom_llm_provider, "watsonx_text");
    assert_eq!(sent[0].api_base.as_deref(), Some("https://ctx.example.com"));
    assert_eq!(
        exposed(sent[0].api_key.as_ref()).as_deref(),
        Some("ctx-key")
    );
    assert_eq!(
        sent[0].optional_params,
        object(
            r#"{"apikey":"param-key","url":"https://url.example.com","base_url":"https://legacy.example.com"}"#
        )
    );
}

#[test]
fn watsonx_text_pops_the_parameter_keys_and_reads_the_environment() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.optional_params = object(
        r#"{"apikey":"param-key","base_url":"https://legacy.example.com","temperature":0.2}"#,
    );
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    chat::dispatch::<String>(
        ChatConfigTag::IbmWatsonXAi,
        &chat.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the text arm runs")
    .expect("the handler answers");
    let sent = recorder.calls();
    assert_eq!(
        sent[0].api_base.as_deref(),
        Some("https://legacy.example.com")
    );
    assert_eq!(
        exposed(sent[0].api_key.as_ref()).as_deref(),
        Some("param-key")
    );
    assert_eq!(sent[0].optional_params, object(r#"{"temperature":0.2}"#));

    let mut environment = Chat::new("watsonx_text", "ibm/granite");
    environment.environment = BTreeMap::from([
        (String::from("WATSONX_APIKEY"), String::new()),
        (String::from("WATSONX_API_KEY"), String::from("second-key")),
        (String::from("WATSONX_API_BASE"), String::new()),
        (
            String::from("WATSONX_URL"),
            String::from("https://env.example.com"),
        ),
        (
            String::from("WX_URL"),
            String::from("https://wx-url.example.com"),
        ),
    ]);
    let recorder = Recorder::answer("reply");
    chat::dispatch::<String>(
        ChatConfigTag::IbmWatsonXAi,
        &environment.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the text arm runs")
    .expect("the handler answers");
    let sent = recorder.calls();
    assert_eq!(
        exposed(sent[0].api_key.as_ref()).as_deref(),
        Some("second-key")
    );
    assert_eq!(sent[0].api_base.as_deref(), Some("https://env.example.com"));
}

#[test]
fn watsonx_text_prefers_url_and_removes_all_three_bases() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.optional_params = object(
        r#"{"url":"https://url.example.com","api_base":"https://api-base.example.com","base_url":"https://base-url.example.com"}"#,
    );
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    chat::dispatch::<String>(
        ChatConfigTag::IbmWatsonXAi,
        &chat.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the text arm runs")
    .expect("the handler answers");
    let sent = recorder.calls();
    assert_eq!(sent[0].api_base.as_deref(), Some("https://url.example.com"));
    assert!(sent[0].optional_params.is_empty());
}

#[test]
fn watsonx_text_applies_the_wx_credentials_over_the_nested_ones() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.optional_params = object(
        r#"{"watsonx_credentials":{"url":"https://nested.example.com","apikey":"nested-key","token":"nested-token"},"wx_credentials":{"url":"https://wx.example.com","api_key":"wx-key","watsonx_token":"wx-token"}}"#,
    );
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    chat::dispatch::<String>(
        ChatConfigTag::IbmWatsonXAi,
        &chat.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the text arm runs")
    .expect("the handler answers");
    let sent = recorder.calls();
    assert_eq!(sent[0].api_base.as_deref(), Some("https://wx.example.com"));
    assert_eq!(exposed(sent[0].api_key.as_ref()).as_deref(), Some("wx-key"));
    assert_eq!(sent[0].optional_params, object(r#"{"token":"wx-token"}"#));

    let mut nested_only = Chat::new("watsonx_text", "ibm/granite");
    nested_only.optional_params = object(
        r#"{"watsonx_credentials":{"url":"https://nested.example.com","apikey":"nested-key","watsonx_token":"nested-token"}}"#,
    );
    let recorder = Recorder::answer("reply");
    chat::dispatch::<String>(
        ChatConfigTag::IbmWatsonXAi,
        &nested_only.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the text arm runs")
    .expect("the handler answers");
    let sent = recorder.calls();
    assert_eq!(
        sent[0].api_base.as_deref(),
        Some("https://nested.example.com")
    );
    assert_eq!(
        exposed(sent[0].api_key.as_ref()).as_deref(),
        Some("nested-key")
    );
    assert_eq!(
        sent[0].optional_params,
        object(r#"{"token":"nested-token"}"#)
    );
}

#[test]
fn watsonx_text_keeps_a_present_null_wx_from_falling_back() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.api_base = Some(String::from("https://ctx.example.com"));
    chat.api_key = Some(key("ctx-key"));
    chat.optional_params = object(
        r#"{"wx_credentials":null,"watsonx_credentials":{"url":"https://nested.example.com","apikey":"nested-key"}}"#,
    );
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    chat::dispatch::<String>(
        ChatConfigTag::IbmWatsonXAi,
        &chat.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the text arm runs")
    .expect("the handler answers");
    let sent = recorder.calls();
    assert_eq!(sent[0].api_base.as_deref(), Some("https://ctx.example.com"));
    assert_eq!(
        exposed(sent[0].api_key.as_ref()).as_deref(),
        Some("ctx-key")
    );
    assert!(sent[0].optional_params.is_empty());
}

#[test]
fn watsonx_text_rejects_a_typed_key() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.optional_params = object(r#"{"apikey":5}"#);
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    let error = chat::dispatch::<String>(
        ChatConfigTag::IbmWatsonXAi,
        &chat.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the text arm runs")
    .expect_err("the key is not a string");
    assert!(matches!(
        error,
        EnterpriseError::FieldType { field: "apikey" }
    ));
    assert!(recorder.calls().is_empty());

    let mut nested = Chat::new("watsonx_text", "ibm/granite");
    nested.optional_params = object(r#"{"wx_credentials":{"url":5}}"#);
    let error = chat::dispatch::<String>(
        ChatConfigTag::IbmWatsonXAi,
        &nested.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the text arm runs")
    .expect_err("the url is not a string");
    assert!(matches!(error, EnterpriseError::FieldType { field: "url" }));
}

#[test]
fn sagemaker_chat_keeps_the_context_provider_name() {
    let mut chat = Chat::new("sagemaker_chat", "sagemaker_chat/endpoint");
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    chat::dispatch::<String>(
        ChatConfigTag::SagemakerChat,
        &chat.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the sagemaker chat arm runs")
    .expect("the handler answers");
    assert_eq!(recorder.calls()[0].custom_llm_provider, "sagemaker_chat");

    chat.provider = "sagemaker_nova";
    chat.model = String::from("sagemaker_nova/endpoint");
    chat::dispatch::<String>(
        ChatConfigTag::SagemakerNova,
        &chat.context(),
        &sagemaker.deps(),
        &recorder,
    )
    .expect("the nova arm runs")
    .expect("the handler answers");
    assert_eq!(recorder.calls()[1].custom_llm_provider, "sagemaker_nova");
}

#[test]
fn sagemaker_completion_builds_the_signed_hf_call() {
    let mut chat = Chat::new("sagemaker", "sagemaker/endpoint");
    chat.provider_config = Some(ChatConfigTag::Sagemaker);
    let sagemaker = Sagemaker::new(Scripted::new(vec![(200, r#"{"generated_text":"hello"}"#)]));
    let outcome = chat::dispatch::<String>(
        ChatConfigTag::Sagemaker,
        &chat.context(),
        &sagemaker.deps(),
        &Recorder::answer("unused"),
    )
    .expect("the sagemaker arm runs")
    .expect("the runtime answers");
    let Completion::Sagemaker(SagemakerReply::Completion(response)) = outcome else {
        panic!("a completion reply is expected");
    };
    assert_eq!(
        response.choices[0].message.content.as_deref(),
        Some("hello")
    );
    assert_eq!(response.model, "sagemaker/endpoint");
    assert_eq!(response.id, format!("chatcmpl-{}", identifier()));
    let sent = sagemaker.transport.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(
        sent[0].url,
        "https://runtime.sagemaker.us-west-2.amazonaws.com/endpoints/endpoint/invocations"
    );
    assert!(
        sagemaker
            .transport
            .header(0, http::header::AUTHORIZATION)
            .is_some_and(|value| value.starts_with("AWS4-HMAC-SHA256"))
    );
    assert_eq!(
        sagemaker.transport.body(0)["inputs"],
        serde_json::json!("hi")
    );
}

#[test]
fn sagemaker_completion_streams_when_the_flag_is_set() {
    let mut chat = Chat::new("sagemaker", "sagemaker/endpoint");
    chat.optional_params = object(r#"{"stream":true}"#);
    let sagemaker = Sagemaker::new(Scripted::empty());
    let outcome = chat::dispatch::<String>(
        ChatConfigTag::Sagemaker,
        &chat.context(),
        &sagemaker.deps(),
        &Recorder::answer("unused"),
    )
    .expect("the sagemaker arm runs");
    assert!(outcome.is_err());
    assert!(sagemaker.transport.streamed());
    let sent = sagemaker.transport.sent();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].url.ends_with("/invocations-response-stream"),
        "the stream URL is expected, got {}",
        sent[0].url
    );
}

#[test]
fn sagemaker_completion_accepts_a_message_without_content() {
    let mut chat = Chat::new("sagemaker", "sagemaker/endpoint");
    chat.messages = vec![message(r#"{"role":"user"}"#)];
    chat.sagemaker_prompts = Some(BTreeMap::from([(
        String::from("sagemaker/endpoint"),
        CustomPromptEntry {
            roles: BTreeMap::new(),
            initial_prompt_value: String::from("Q:"),
            final_prompt_value: String::new(),
        },
    )]));
    let sagemaker = Sagemaker::new(Scripted::new(vec![(200, r#"{"generated_text":"hello"}"#)]));
    chat::dispatch::<String>(
        ChatConfigTag::Sagemaker,
        &chat.context(),
        &sagemaker.deps(),
        &Recorder::answer("unused"),
    )
    .expect("the sagemaker arm runs")
    .expect("the null content still builds a prompt");
    assert_eq!(
        sagemaker.transport.body(0)["inputs"],
        serde_json::json!("Q:")
    );
}

#[test]
fn embedding_configuration_picks_the_three_arms() {
    assert_eq!(
        embedding_config(ProviderId::SapGenerativeAiHub),
        Some(EmbeddingConfig::Sap)
    );
    assert_eq!(
        embedding_config(ProviderId::Watsonx),
        Some(EmbeddingConfig::Watsonx)
    );
    assert_eq!(
        embedding_config(ProviderId::Sagemaker),
        Some(EmbeddingConfig::Sagemaker)
    );
    assert_eq!(embedding_config(ProviderId::Openai), None);
}

#[test]
fn rerank_configuration_picks_watsonx_only() {
    assert_eq!(
        rerank_config(ProviderId::Watsonx),
        Some(RerankConfig::Watsonx)
    );
    assert_eq!(rerank_config(ProviderId::Sagemaker), None);
    assert_eq!(rerank_config(ProviderId::Openai), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn sap_embedding_runs_the_token_and_deployment_flow() {
    let transport = Scripted::new(vec![
        (200, r#"{"access_token":"oracle-token","expires_in":7200}"#),
        (
            200,
            r#"{"resources":[
                {"scenarioId":"other","configurationId":"skip","deploymentUrl":"https://other.example.com","createdAt":"2024-07-01"},
                {"scenarioId":"orchestration","configurationId":"old","deploymentUrl":"https://old.example.com","createdAt":"2024-01-01"},
                {"scenarioId":"orchestration","configurationId":"new","deploymentUrl":"https://dep.example.com/","createdAt":"2024-06-01"}
            ]}"#,
        ),
        (200, r#"{"executableId":"orchestration"}"#),
        (200, r#"{"executableId":"orchestration"}"#),
        (
            200,
            r#"{"final_result":{"object":"list","data":[{"object":"embedding","embedding":[0.1,0.2],"index":0}],"model":"text-embedding-3-small","usage":{"prompt_tokens":2,"total_tokens":2}}}"#,
        ),
    ]);
    let secrets = secrets();
    let environment = sap_credentials_environment();
    let env = EnvironmentSource {
        secrets: &secrets,
        environment: &environment,
    };
    let sagemaker = Sagemaker::new(Scripted::empty());
    let clock = FixedClock;
    let context = EmbeddingContext {
        env,
        sap_service_key: None,
        transport: &transport,
        sagemaker: sagemaker.deps(),
        clock: &clock,
    };
    let call = embedding_call("text-embedding-3-small", Some(key(SAP_SERVICE_KEY)));
    let reply = embed::dispatch(&call, EmbeddingConfig::Sap, &context)
        .await
        .expect("the sap embedding resolves");
    let EmbeddingReply::Sap(response) = reply else {
        panic!("a sap reply is expected");
    };
    assert_eq!(response.data.len(), 1);
    assert_eq!(
        response.data[0].embedding,
        EmbeddingVector::Floats(vec![
            serde_json::Number::from_f64(0.1).expect("a fixture number"),
            serde_json::Number::from_f64(0.2).expect("a fixture number"),
        ])
    );
    assert_eq!(response.usage.prompt_tokens, 2);

    let sent = transport.sent();
    assert_eq!(sent.len(), 5);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].url, "https://auth.example.com/oauth/token");
    assert_eq!(
        String::from_utf8(sent[0].body.clone()).expect("the form is text"),
        "grant_type=client_credentials&client_id=client&client_secret=secret"
    );
    assert_eq!(sent[1].method, "GET");
    assert_eq!(sent[1].url, "https://base.example.com/v2/lm/deployments");
    assert_eq!(sent[2].method, "GET");
    assert_eq!(
        sent[2].url,
        "https://base.example.com/v2/lm/configurations/old"
    );
    assert_eq!(sent[3].method, "GET");
    assert_eq!(
        sent[3].url,
        "https://base.example.com/v2/lm/configurations/new"
    );
    assert_eq!(sent[4].method, "POST");
    assert_eq!(sent[4].url, "https://dep.example.com/v2/embeddings");
    for index in 1..=4 {
        assert_eq!(
            transport
                .header(index, http::header::AUTHORIZATION)
                .as_deref(),
            Some("Bearer oracle-token")
        );
        assert_eq!(
            transport
                .header(
                    index,
                    http::header::HeaderName::from_static("ai-resource-group")
                )
                .as_deref(),
            Some("default")
        );
        assert_eq!(
            transport
                .header(
                    index,
                    http::header::HeaderName::from_static("ai-client-type")
                )
                .as_deref(),
            Some("LiteLLM")
        );
    }
    let body = transport.body(4);
    assert_eq!(body["input"]["text"], serde_json::json!("hi"));
    assert_eq!(
        body["config"]["modules"]["embeddings"]["model"]["name"],
        serde_json::json!("text-embedding-3-small")
    );
    assert_eq!(
        body["config"]["modules"]["embeddings"]["model"]["version"],
        serde_json::json!("latest")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sap_chat_prepare_runs_the_chat_deployment_lookup() {
    let transport = Scripted::new(vec![
        (200, r#"{"access_token":"oracle-token","expires_in":7200}"#),
        (
            200,
            r#"{"resources":[{"scenarioId":"orchestration","configurationId":"cfg","deploymentUrl":"https://dep.example.com/","createdAt":"2024-06-01"}]}"#,
        ),
        (200, r#"{"executableId":"orchestration"}"#),
    ]);
    let secrets = secrets();
    let environment = sap_credentials_environment();
    let env = EnvironmentSource {
        secrets: &secrets,
        environment: &environment,
    };
    let clock = FixedClock;
    let prepared = sap::prepare_chat(
        SapRequest {
            api_key: None,
            sap_service_key: None,
            env,
        },
        &transport,
        &clock,
    )
    .expect("the chat session resolves");
    assert_eq!(prepared.url, "https://dep.example.com/v2/completion");
    assert_eq!(
        prepared
            .headers
            .get(http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer oracle-token")
    );
    assert_eq!(transport.sent().len(), 3);

    let empty = sap_environment(&[]);
    let env = EnvironmentSource {
        secrets: &secrets,
        environment: &empty,
    };
    let error = sap::prepare_chat(
        SapRequest {
            api_key: None,
            sap_service_key: None,
            env,
        },
        &transport,
        &clock,
    )
    .expect_err("no credentials are set");
    assert!(matches!(
        error,
        EnterpriseError::SapChat(SapChatError::Orchestration(_))
    ));
    assert_eq!(error.status(), Some(400));
}

#[tokio::test(flavor = "multi_thread")]
async fn sap_embedding_reports_missing_credentials() {
    let transport = Scripted::empty();
    let secrets = secrets();
    let environment = sap_environment(&[]);
    let env = EnvironmentSource {
        secrets: &secrets,
        environment: &environment,
    };
    let sagemaker = Sagemaker::new(Scripted::empty());
    let clock = FixedClock;
    let context = EmbeddingContext {
        env,
        sap_service_key: None,
        transport: &transport,
        sagemaker: sagemaker.deps(),
        clock: &clock,
    };
    let call = embedding_call("text-embedding-3-small", None);
    let error = embed::dispatch(&call, EmbeddingConfig::Sap, &context)
        .await
        .expect_err("no credentials are set");
    assert!(matches!(
        error,
        EnterpriseError::SapEmbedding(SapEmbeddingError::Auth(SapAuthError::NoCredentials))
    ));
    assert_eq!(error.status(), None);
    assert!(transport.sent().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn sap_embedding_refuses_a_client_certificate() {
    let transport = Scripted::empty();
    let secrets = secrets();
    let environment = sap_environment(&[
        ("AICORE_CLIENT_ID", "client"),
        ("AICORE_AUTH_URL", "https://auth.example.com"),
        ("AICORE_BASE_URL", "https://base.example.com"),
        ("AICORE_CERT_STR", "cert"),
        ("AICORE_KEY_STR", "key"),
    ]);
    let env = EnvironmentSource {
        secrets: &secrets,
        environment: &environment,
    };
    let sagemaker = Sagemaker::new(Scripted::empty());
    let clock = FixedClock;
    let context = EmbeddingContext {
        env,
        sap_service_key: None,
        transport: &transport,
        sagemaker: sagemaker.deps(),
        clock: &clock,
    };
    let call = embedding_call("text-embedding-3-small", None);
    let error = embed::dispatch(&call, EmbeddingConfig::Sap, &context)
        .await
        .expect_err("the pooled client holds no certificate");
    assert!(
        error.to_string().contains(
            "client certificate authentication is not supported by the pooled SAP client"
        )
    );
    assert_eq!(error.status(), None);
    assert!(transport.sent().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn sap_embedding_reports_a_non_success_reply() {
    let transport = Scripted::new(vec![
        (200, r#"{"access_token":"oracle-token","expires_in":7200}"#),
        (
            200,
            r#"{"resources":[{"scenarioId":"orchestration","configurationId":"new","deploymentUrl":"https://dep.example.com","createdAt":"2024-06-01"}]}"#,
        ),
        (200, r#"{"executableId":"orchestration"}"#),
        (400, r#"{"error":"bad"}"#),
    ]);
    let secrets = secrets();
    let environment = sap_credentials_environment();
    let env = EnvironmentSource {
        secrets: &secrets,
        environment: &environment,
    };
    let sagemaker = Sagemaker::new(Scripted::empty());
    let clock = FixedClock;
    let context = EmbeddingContext {
        env,
        sap_service_key: None,
        transport: &transport,
        sagemaker: sagemaker.deps(),
        clock: &clock,
    };
    let call = embedding_call("text-embedding-3-small", None);
    let error = embed::dispatch(&call, EmbeddingConfig::Sap, &context)
        .await
        .expect_err("the orchestration service rejects the embedding");
    assert!(matches!(error, EnterpriseError::SapOrchestration(_)));
    assert_eq!(error.status(), Some(400));
    assert_eq!(error.to_string(), r#"{"error":"bad"}"#);
}

#[tokio::test(flavor = "multi_thread")]
async fn watsonx_embedding_posts_the_project_request() {
    let transport = Scripted::new(vec![(
        200,
        r#"{"results":[{"embedding":[0.1,0.2]}],"input_token_count":2}"#,
    )]);
    let secrets = secrets();
    let environment = BTreeMap::from([
        (String::from("WATSONX_TOKEN"), String::from("env-token")),
        (String::from("WATSONX_PROJECT_ID"), String::from("project")),
    ]);
    let env = EnvironmentSource {
        secrets: &secrets,
        environment: &environment,
    };
    let sagemaker = Sagemaker::new(Scripted::empty());
    let clock = FixedClock;
    let context = EmbeddingContext {
        env,
        sap_service_key: None,
        transport: &transport,
        sagemaker: sagemaker.deps(),
        clock: &clock,
    };
    let mut call = embedding_call("ibm/granite-embedding", Some(key("api-key")));
    call.api_base = Some(String::from("https://wx.example.com"));
    let reply = embed::dispatch(&call, EmbeddingConfig::Watsonx, &context)
        .await
        .expect("the watsonx embedding resolves");
    let EmbeddingReply::Watsonx(response) = reply else {
        panic!("a watsonx reply is expected");
    };
    assert_eq!(response.model, "ibm/granite-embedding");
    assert_eq!(response.data.len(), 1);
    assert_eq!(response.usage.prompt_tokens, 2);
    assert_eq!(response.usage.total_tokens, 2);
    let sent = transport.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(
        sent[0].url,
        "https://wx.example.com/ml/v1/text/embeddings?version=2024-03-13"
    );
    assert_eq!(
        transport.header(0, http::header::AUTHORIZATION).as_deref(),
        Some("Bearer env-token")
    );
    let body = transport.body(0);
    assert_eq!(body["inputs"], serde_json::json!(["hi"]));
    assert_eq!(body["parameters"], serde_json::json!({}));
    assert_eq!(body["model_id"], serde_json::json!("ibm/granite-embedding"));
    assert_eq!(body["project_id"], serde_json::json!("project"));
}

#[tokio::test(flavor = "multi_thread")]
async fn watsonx_embedding_without_ids_reports_401() {
    let transport = Scripted::empty();
    let secrets = secrets();
    let environment = BTreeMap::from([(String::from("WATSONX_TOKEN"), String::from("env-token"))]);
    let env = EnvironmentSource {
        secrets: &secrets,
        environment: &environment,
    };
    let sagemaker = Sagemaker::new(Scripted::empty());
    let clock = FixedClock;
    let context = EmbeddingContext {
        env,
        sap_service_key: None,
        transport: &transport,
        sagemaker: sagemaker.deps(),
        clock: &clock,
    };
    let mut call = embedding_call("ibm/granite-embedding", Some(key("api-key")));
    call.api_base = Some(String::from("https://wx.example.com"));
    let error = embed::dispatch(&call, EmbeddingConfig::Watsonx, &context)
        .await
        .expect_err("no project or space is set");
    assert!(matches!(
        error,
        EnterpriseError::WatsonxEmbedding(WatsonxEmbeddingError::Watsonx(ref inner))
            if matches!(inner.as_ref(), WatsonxError::Api { status_code: 401, .. })
    ));
    assert_eq!(error.status(), Some(401));
    assert!(
        error
            .to_string()
            .contains("project_id and space_id not set")
    );
    assert!(transport.sent().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn watsonx_embedding_reports_a_non_success_reply() {
    let transport = Scripted::new(vec![(400, r#"{"errors":[{"code":"bad"}]}"#)]);
    let secrets = secrets();
    let environment = BTreeMap::from([
        (String::from("WATSONX_TOKEN"), String::from("env-token")),
        (String::from("WATSONX_PROJECT_ID"), String::from("project")),
    ]);
    let env = EnvironmentSource {
        secrets: &secrets,
        environment: &environment,
    };
    let sagemaker = Sagemaker::new(Scripted::empty());
    let clock = FixedClock;
    let context = EmbeddingContext {
        env,
        sap_service_key: None,
        transport: &transport,
        sagemaker: sagemaker.deps(),
        clock: &clock,
    };
    let mut call = embedding_call("ibm/granite-embedding", Some(key("api-key")));
    call.api_base = Some(String::from("https://wx.example.com"));
    let error = embed::dispatch(&call, EmbeddingConfig::Watsonx, &context)
        .await
        .expect_err("the embedding service rejects the request");
    assert_eq!(error.status(), Some(400));
    assert_eq!(error.to_string(), r#"{"errors":[{"code":"bad"}]}"#);
}

#[tokio::test(flavor = "multi_thread")]
async fn sagemaker_embedding_posts_the_hf_request() {
    let transport = Scripted::new(vec![(200, "[[0.1,0.2]]")]);
    let secrets = secrets();
    let environment = BTreeMap::new();
    let env = EnvironmentSource {
        secrets: &secrets,
        environment: &environment,
    };
    let unused = Scripted::empty();
    let sagemaker = Sagemaker::new(transport);
    let clock = FixedClock;
    let context = EmbeddingContext {
        env,
        sap_service_key: None,
        transport: &unused,
        sagemaker: sagemaker.deps(),
        clock: &clock,
    };
    let call = embedding_call("sagemaker/endpoint", None);
    let reply = embed::dispatch(&call, EmbeddingConfig::Sagemaker, &context)
        .await
        .expect("the sagemaker embedding resolves");
    assert!(matches!(reply, EmbeddingReply::Sagemaker(_)));
    let sent = sagemaker.transport.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(
        sent[0].url,
        "https://runtime.sagemaker.us-west-2.amazonaws.com/endpoints/endpoint/invocations"
    );
    assert!(
        sagemaker
            .transport
            .header(0, http::header::AUTHORIZATION)
            .is_some_and(|value| value.starts_with("AWS4-HMAC-SHA256"))
    );
    assert_eq!(
        sagemaker.transport.body(0)["inputs"],
        serde_json::json!(["hi"])
    );
}

#[test]
fn tokenizer_counts_tokens() {
    let tokenizer = Tokenizer::new().expect("the bundled vocabulary loads");
    assert_eq!(tokenizer.count_response_tokens("hello world"), 2);
    assert_eq!(
        tokenizer
            .count_tokens("hello world")
            .expect("the engine encodes"),
        2
    );
}

#[test]
fn enterprise_error_carries_the_provider_status() {
    let handler = EnterpriseError::Handler(HandlerError::Upstream { status_code: 502 });
    assert_eq!(handler.status(), Some(502));
    let chat_credentials =
        EnterpriseError::SapChat(SapChatError::credentials(SapAuthError::NoCredentials));
    assert_eq!(chat_credentials.status(), Some(400));
    let embedding_credentials =
        EnterpriseError::SapEmbedding(SapEmbeddingError::Auth(SapAuthError::MissingCredentials));
    assert_eq!(embedding_credentials.status(), None);
    let orchestration = EnterpriseError::SapOrchestration(SapOrchestrationError {
        status_code: 418,
        message: String::from("teapot"),
    });
    assert_eq!(orchestration.status(), Some(418));
    let watsonx = EnterpriseError::Watsonx(WatsonxError::Api {
        status_code: 401,
        message: String::from("no key"),
        headers: None,
    });
    assert_eq!(watsonx.status(), Some(401));
    let sagemaker = EnterpriseError::Sagemaker(SagemakerError {
        status_code: 500,
        message: String::from("boom"),
        headers: None,
    });
    assert_eq!(sagemaker.status(), Some(500));
    let typed = EnterpriseError::FieldType { field: "apikey" };
    assert_eq!(typed.status(), None);
}

#[test]
fn watsonx_environment_reads_every_name() {
    let secrets = secrets();
    let environment = BTreeMap::from([
        (
            String::from("WATSONX_IAM_URL"),
            String::from("https://iam.example.com"),
        ),
        (String::from("WX_API_KEY"), String::from("wx-key")),
        (String::from("WATSONX_API_KEY"), String::from("api-key")),
        (String::from("WATSONX_APIKEY"), String::from("apikey")),
        (String::from("WATSONX_ZENAPIKEY"), String::from("zen")),
        (String::from("WATSONX_TOKEN"), String::from("token")),
        (
            String::from("WATSONX_API_BASE"),
            String::from("https://api-base.example.com"),
        ),
        (
            String::from("WATSONX_URL"),
            String::from("https://url.example.com"),
        ),
        (
            String::from("WX_URL"),
            String::from("https://wx-url.example.com"),
        ),
        (
            String::from("WML_URL"),
            String::from("https://wml.example.com"),
        ),
        (String::from("WATSONX_PROJECT_ID"), String::from("p1")),
        (String::from("WX_PROJECT_ID"), String::from("p2")),
        (String::from("PROJECT_ID"), String::from("p3")),
        (String::from("WATSONX_REGION"), String::from("r1")),
        (String::from("WX_REGION"), String::from("r2")),
        (String::from("REGION"), String::from("r3")),
        (
            String::from("WATSONX_DEPLOYMENT_SPACE_ID"),
            String::from("s1"),
        ),
        (String::from("WATSONX_SPACE_ID"), String::from("s2")),
        (String::from("WX_SPACE_ID"), String::from("s3")),
        (String::from("SPACE_ID"), String::from("s4")),
    ]);
    let env = EnvironmentSource {
        secrets: &secrets,
        environment: &environment,
    };
    let filled = watsonx::environment(&env).expect("the environment reads");
    assert_eq!(
        filled.watsonx_iam_url.as_deref(),
        Some("https://iam.example.com")
    );
    assert_eq!(
        exposed(filled.wx_api_key.as_ref()).as_deref(),
        Some("wx-key")
    );
    assert_eq!(
        exposed(filled.watsonx_api_key.as_ref()).as_deref(),
        Some("api-key")
    );
    assert_eq!(
        exposed(filled.watsonx_apikey.as_ref()).as_deref(),
        Some("apikey")
    );
    assert_eq!(
        exposed(filled.watsonx_zenapikey.as_ref()).as_deref(),
        Some("zen")
    );
    assert_eq!(
        exposed(filled.watsonx_token.as_ref()).as_deref(),
        Some("token")
    );
    assert_eq!(
        filled.watsonx_api_base.as_deref(),
        Some("https://api-base.example.com")
    );
    assert_eq!(
        filled.watsonx_url.as_deref(),
        Some("https://url.example.com")
    );
    assert_eq!(filled.wx_url.as_deref(), Some("https://wx-url.example.com"));
    assert_eq!(filled.wml_url.as_deref(), Some("https://wml.example.com"));
    assert_eq!(filled.watsonx_project_id.as_deref(), Some("p1"));
    assert_eq!(filled.wx_project_id.as_deref(), Some("p2"));
    assert_eq!(filled.project_id.as_deref(), Some("p3"));
    assert_eq!(filled.watsonx_region.as_deref(), Some("r1"));
    assert_eq!(filled.wx_region.as_deref(), Some("r2"));
    assert_eq!(filled.region.as_deref(), Some("r3"));
    assert_eq!(filled.watsonx_deployment_space_id.as_deref(), Some("s1"));
    assert_eq!(filled.watsonx_space_id.as_deref(), Some("s2"));
    assert_eq!(filled.wx_space_id.as_deref(), Some("s3"));
    assert_eq!(filled.space_id.as_deref(), Some("s4"));

    let empty = BTreeMap::from([(String::from("WATSONX_APIKEY"), String::new())]);
    let env = EnvironmentSource {
        secrets: &secrets,
        environment: &empty,
    };
    let blank = watsonx::environment(&env).expect("the environment reads");
    assert!(blank.watsonx_apikey.is_none());
    assert!(blank.watsonx_api_base.is_none());
}
