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
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::sagemaker_completion::request::CustomPromptEntry;
use betterlite_openai::sagemaker_completion::response::TextTokenCounter;
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
use crate::enterprise_ml_dispatch::call::SagemakerRequest;
use crate::enterprise_ml_dispatch::chat::{self, Completion};
use crate::enterprise_ml_dispatch::context::ChatContext;
use crate::enterprise_ml_dispatch::embed::{self, EmbeddingContext, EmbeddingReply};
use crate::enterprise_ml_dispatch::error::EnterpriseError;
use crate::enterprise_ml_dispatch::sagemaker::SagemakerDeps;
use crate::enterprise_ml_dispatch::select::EmbeddingConfig;
use crate::sap_auth::token::SapClock;

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

pub(super) fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(Offline),
        azure_tokens: Arc::new(Offline),
        clock: Arc::new(Offline),
        oidc_cache: OidcTokenCache::default(),
    }
}

// the secret store and the environment snapshot a call runs with
pub(super) struct Env {
    pub(super) secrets: SecretLookup,
    pub(super) environment: BTreeMap<String, String>,
}

impl Env {
    pub(super) fn new(environment: BTreeMap<String, String>) -> Self {
        Self {
            secrets: secrets(),
            environment,
        }
    }

    pub(super) fn source(&self) -> EnvironmentSource<'_> {
        EnvironmentSource {
            secrets: &self.secrets,
            environment: &self.environment,
        }
    }
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn key(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(super) fn object(wire: &str) -> Object {
    serde_json::from_str(wire).expect("the fixture object is JSON")
}

pub(super) fn message(wire: &str) -> Message {
    serde_json::from_str(wire).expect("the fixture message is a chat message")
}

pub(super) fn exposed(value: Option<&SecretString>) -> Option<String> {
    value.map(|stored| stored.expose_secret().to_owned())
}

// walks a recorded JSON body along an object path
pub(super) fn at<'json>(root: &'json Json, path: &[&str]) -> &'json Json {
    let mut current = root;
    for name in path {
        current = current
            .as_object()
            .and_then(|fields| fields.get(*name))
            .expect("the recorded body carries the path");
    }
    current
}

#[derive(Clone)]
pub(super) struct Sent {
    pub(super) method: String,
    pub(super) url: String,
    pub(super) headers: http::HeaderMap,
    pub(super) body: Vec<u8>,
}

fn response(status: u16, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: http::HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

// the pooled client, answering the scripted replies in arrival order and recording every request
pub(super) struct Scripted {
    replies: Mutex<VecDeque<UpstreamResponse>>,
    sent: Mutex<Vec<Sent>>,
    streamed: Mutex<bool>,
}

impl Scripted {
    pub(super) fn new(replies: Vec<(u16, &str)>) -> Self {
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

    pub(super) fn empty() -> Self {
        Self::new(Vec::new())
    }

    pub(super) fn sent(&self) -> Vec<Sent> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn streamed(&self) -> bool {
        *self.streamed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn body(&self, index: usize) -> Json {
        let sent = self.sent();
        serde_json::from_slice(&sent[index].body).expect("the fixture recorded JSON")
    }

    pub(super) fn header(&self, index: usize, name: http::header::HeaderName) -> Option<String> {
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

pub(super) struct StubCredentials;

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

pub(super) struct Counter;

impl TextTokenCounter for Counter {
    fn count_response_tokens(&self, text: &str) -> u64 {
        u64::try_from(text.split_whitespace().count()).unwrap_or_default()
    }
}

pub(super) struct NeverEncoding;

impl TextEncoding for NeverEncoding {
    fn count_tokens(&self, _text: &str) -> Result<usize, EngineError> {
        Err(EngineError::PoolClosed)
    }
}

pub(super) fn identifier() -> Uuid {
    Uuid::from_u128(7)
}

fn prompt_factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(BTreeMap::new()),
        hub: Hub::Unreachable,
    }
}

pub(super) struct Sagemaker {
    pub(super) transport: Scripted,
    credentials: StubCredentials,
    counter: Counter,
    encoding: NeverEncoding,
    prompts: PromptFactory,
}

impl Sagemaker {
    pub(super) fn new(transport: Scripted) -> Self {
        Self {
            transport,
            credentials: StubCredentials,
            counter: Counter,
            encoding: NeverEncoding,
            prompts: prompt_factory(),
        }
    }

    pub(super) fn deps(&self) -> SagemakerDeps<'_> {
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

// the context an arm reads, with the sagemaker values the call carries outside it
pub(super) struct Chat {
    pub(super) provider: &'static str,
    pub(super) model: String,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
    pub(super) headers: Option<BTreeMap<String, String>>,
    pub(super) optional_params: Object,
    pub(super) messages: Vec<Message>,
    pub(super) litellm_params: Object,
    pub(super) globals_headers: Option<BTreeMap<String, String>>,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) sagemaker_prompts: Option<BTreeMap<String, CustomPromptEntry>>,
    secrets: SecretLookup,
}

impl Chat {
    pub(super) fn new(provider: &'static str, model: &str) -> Self {
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
            environment: BTreeMap::new(),
            sagemaker_prompts: None,
            secrets: secrets(),
        }
    }

    pub(super) fn context(&self) -> ChatContext<'_> {
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
                provider_config: None,
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

// dispatch with the fixture's context, keeping the outcome concrete for the assertions
pub(super) fn dispatch(
    tag: ChatConfigTag,
    chat: &Chat,
    sagemaker: &Sagemaker,
    recorder: &Recorder,
) -> Option<Result<Completion<String>, EnterpriseError>> {
    chat::dispatch::<String>(tag, &chat.context(), &sagemaker.deps(), recorder)
}

pub(super) struct Recorder {
    calls: Mutex<Vec<CompletionCall>>,
    reply: String,
    failure: Option<u16>,
}

impl Recorder {
    pub(super) fn answer(reply: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            reply: reply.to_owned(),
            failure: None,
        }
    }

    pub(super) fn failing(status_code: u16) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            reply: String::new(),
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

pub(super) struct FixedClock;

impl SapClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("the fixture time converts")
    }
}

pub(super) fn sap_environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    let mut environment = environment(pairs);
    environment.insert(String::from("AICORE_HOME"), String::from(NO_HOME));
    environment
}

pub(super) fn sap_credentials_environment() -> BTreeMap<String, String> {
    sap_environment(&[
        ("AICORE_CLIENT_ID", "client"),
        ("AICORE_CLIENT_SECRET", "secret"),
        ("AICORE_AUTH_URL", "https://auth.example.com"),
        ("AICORE_BASE_URL", "https://base.example.com"),
    ])
}

pub(super) fn embedding_call(model: &str, api_key: Option<SecretString>) -> EmbeddingCall {
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

// the embedding arms with the fixture's environment, transport and sagemaker deps
pub(super) async fn dispatch_embedding(
    call: &EmbeddingCall,
    config: EmbeddingConfig,
    transport: &Scripted,
    env: &Env,
) -> Result<EmbeddingReply, EnterpriseError> {
    let sagemaker = Sagemaker::new(Scripted::empty());
    let clock = FixedClock;
    let context = EmbeddingContext {
        env: env.source(),
        sap_service_key: None,
        transport,
        sagemaker: sagemaker.deps(),
        clock: &clock,
    };
    embed::dispatch(call, config, &context).await
}

// LiteLLM's service key and credentials for the SAP flow, with the fixture hosts
pub(super) const SAP_SERVICE_KEY: &str = r#"{"client_id":"ignored","client_secret":"ignored","url":"https://other-auth.example.com","serviceurls":{"AI_API_URL":"https://other-base.example.com"}}"#;

pub(super) const SAP_EMBEDDING_REPLIES: &[(u16, &str)] = &[
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
];

pub(super) const SAP_CHAT_REPLIES: &[(u16, &str)] = &[
    (200, r#"{"access_token":"oracle-token","expires_in":7200}"#),
    (
        200,
        r#"{"resources":[{"scenarioId":"orchestration","configurationId":"cfg","deploymentUrl":"https://dep.example.com/","createdAt":"2024-06-01"}]}"#,
    ),
    (200, r#"{"executableId":"orchestration"}"#),
];

const WATSONX_NAMES: &[(&str, &str)] = &[
    ("WATSONX_IAM_URL", "https://iam.example.com"),
    ("WX_API_KEY", "wx-key"),
    ("WATSONX_API_KEY", "api-key"),
    ("WATSONX_APIKEY", "apikey"),
    ("WATSONX_ZENAPIKEY", "zen"),
    ("WATSONX_TOKEN", "token"),
    ("WATSONX_API_BASE", "https://api-base.example.com"),
    ("WATSONX_URL", "https://url.example.com"),
    ("WX_URL", "https://wx-url.example.com"),
    ("WML_URL", "https://wml.example.com"),
    ("WATSONX_PROJECT_ID", "p1"),
    ("WX_PROJECT_ID", "p2"),
    ("PROJECT_ID", "p3"),
    ("WATSONX_REGION", "r1"),
    ("WX_REGION", "r2"),
    ("REGION", "r3"),
    ("WATSONX_DEPLOYMENT_SPACE_ID", "s1"),
    ("WATSONX_SPACE_ID", "s2"),
    ("WX_SPACE_ID", "s3"),
    ("SPACE_ID", "s4"),
];

pub(super) fn watsonx_environment() -> BTreeMap<String, String> {
    environment(WATSONX_NAMES)
}
