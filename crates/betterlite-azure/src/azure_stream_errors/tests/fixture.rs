use std::collections::BTreeMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::text_completion_transport::logging::{SecretHeaders, StreamDefaults};
use betterlite_openai::text_completion_transport::sdk::{ByteStream, SdkError};
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::JsonObject;
use futures_util::stream;
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Builder;
use uuid::Uuid;

use crate::azure_client_secret::SecretResolver;
use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::azure_stream_errors::error::{AzureStreamFailure, Raised, Reported, StreamingError};
use crate::azure_stream_errors::records::{
    ChatStreamTransport, GetStreamClient, PreCallRecord, StreamAnswer, StreamCall, StreamClient,
    StreamHooks, StreamRuntime, StreamSource,
};
use crate::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use crate::endpoints::projection::{CallableProvider, ClientKind, LiteLlmParams};
use crate::oidc_exchange::transport::{Clock, HttpReply, Transport};
use crate::oidc_exchange::{AzureOidcExchange, OidcExchangeInput};

pub const CLOCK: i64 = 1_700_000_000;

pub const fn stream_instant() -> StreamInstant {
    StreamInstant {
        seconds: CLOCK,
        timestamp: 1_700_000_000.0,
    }
}

pub const fn identifier() -> Uuid {
    Uuid::from_u128(7)
}

pub const fn runtime() -> StreamRuntime {
    StreamRuntime {
        now: stream_instant,
        next_id: identifier,
    }
}

#[derive(Clone)]
pub struct RequestRecord {
    pub data: Extra,
    pub timeout: Option<f64>,
}

#[derive(Clone)]
pub struct GetRecord {
    pub api_version: String,
    pub api_base: String,
    pub api_key: Option<String>,
    pub model: String,
    pub is_async: bool,
    pub litellm_params: LiteLlmParams,
    pub client_present: bool,
}

#[derive(Default)]
pub struct ScriptedTransport {
    pub headers: Vec<(String, String)>,
    pub payload: Vec<Result<Vec<u8>, String>>,
    pub failure: Option<Raised>,
    pub requests: Arc<Mutex<Vec<RequestRecord>>>,
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

impl ChatStreamTransport for ScriptedTransport {
    fn create<'call>(
        &'call self,
        data: &'call Extra,
        timeout: Option<f64>,
    ) -> Pin<Box<dyn Future<Output = Result<StreamAnswer, Raised>> + Send + 'call>> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RequestRecord {
                data: data.clone(),
                timeout,
            });
        let failure = self.failure.clone();
        let headers = header_map(&self.headers);
        let items: Vec<Result<Vec<u8>, SdkError>> = self
            .payload
            .iter()
            .map(|item| match item {
                Ok(bytes) => Ok(bytes.clone()),
                Err(message) => Err(SdkError::Other {
                    message: message.clone(),
                }),
            })
            .collect();
        Box::pin(async move {
            if let Some(raised) = failure {
                return Err(raised);
            }
            let body: ByteStream = Box::pin(stream::iter(items));
            Ok(StreamAnswer { headers, body })
        })
    }
}

pub struct ScriptedSource {
    pub kind: ClientKind,
    pub api_key: Option<SecretString>,
    pub transport: Arc<ScriptedTransport>,
    pub get_error: Option<Raised>,
    pub gets: Arc<Mutex<Vec<GetRecord>>>,
}

impl Default for ScriptedSource {
    fn default() -> Self {
        scripted_source(ClientKind::AzureOpenAi, &[], Vec::new())
    }
}

impl StreamSource for ScriptedSource {
    fn get(&self, request: GetStreamClient<'_>) -> Result<StreamClient, Raised> {
        self.gets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(GetRecord {
                api_version: request.api_version.to_owned(),
                api_base: request.api_base.to_owned(),
                api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
                model: request.model.to_owned(),
                is_async: request.is_async,
                litellm_params: request.litellm_params.clone(),
                client_present: request.supplied.is_some(),
            });
        if let Some(error) = &self.get_error {
            return Err(error.clone());
        }
        let transport: Arc<dyn ChatStreamTransport> = Arc::clone(&self.transport);
        Ok(StreamClient {
            kind: self.kind,
            api_key: self.api_key.clone(),
            transport,
        })
    }
}

#[derive(Default)]
pub struct Recorder {
    pub pre: Vec<PreCallRecord>,
    pub response_headers: Vec<IndexMap<String, String>>,
    pub fail_pre: bool,
}

impl StreamHooks for Recorder {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        if self.fail_pre {
            return Err(PrePostError::CallbackFailed {
                message: "pre_call refused".to_owned(),
            });
        }
        self.pre.push(record);
        Ok(())
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

struct FixedClock;

impl Clock for FixedClock {
    fn now_unix_seconds(&self) -> f64 {
        1_700_000_000.0
    }
}

enum RecordedBody {
    Fixed(String),
    FromAssertion,
}

struct RecordingTransport {
    status: u16,
    body: RecordedBody,
    forms: Arc<Mutex<Vec<(String, Vec<(String, String)>)>>>,
}

fn assertion_of(form: &[(String, String)]) -> String {
    form.iter()
        .find(|(name, _)| name == "client_assertion")
        .map(|(_, value)| value.clone())
        .unwrap_or_default()
}

impl Transport for RecordingTransport {
    fn post_form(&self, url: &str, form: &[(String, SecretString)]) -> io::Result<HttpReply> {
        let fields: Vec<(String, String)> = form
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        let body = match &self.body {
            RecordedBody::Fixed(text) => text.clone(),
            RecordedBody::FromAssertion => format!(
                "{{\"access_token\":\"token-{}\",\"expires_in\":3600}}",
                assertion_of(&fields)
            ),
        };
        self.forms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((url.to_owned(), fields));
        Ok(HttpReply {
            status: self.status,
            body: SecretString::from(body),
        })
    }
}

struct FixedResolver {
    assertions: BTreeMap<String, SecretString>,
}

impl SecretResolver for FixedResolver {
    fn get_secret_str(&self, reference: &str) -> Option<SecretString> {
        self.assertions.get(reference).cloned()
    }
}

pub struct ScriptedSources {
    exchange: AzureOidcExchange,
    forms: Arc<Mutex<Vec<(String, Vec<(String, String)>)>>>,
    resolver: FixedResolver,
}

impl ScriptedSources {
    pub fn new(status: u16, body: &str, assertions: BTreeMap<String, SecretString>) -> Self {
        Self::build(status, RecordedBody::Fixed(body.to_owned()), assertions)
    }

    pub fn echoing(assertions: BTreeMap<String, SecretString>) -> Self {
        Self::build(200, RecordedBody::FromAssertion, assertions)
    }

    fn build(status: u16, body: RecordedBody, assertions: BTreeMap<String, SecretString>) -> Self {
        let forms = Arc::new(Mutex::new(Vec::new()));
        let settings = ConfigLoader {
            lookup: BTreeMap::new(),
        }
        .load("")
        .expect("config loads")
        .sections
        .azure_oidc_exchange;
        let environment = BTreeMap::from([
            ("AZURE_CLIENT_ID".to_owned(), "client-A".to_owned()),
            ("AZURE_TENANT_ID".to_owned(), "tenant".to_owned()),
        ]);
        let exchange = AzureOidcExchange::new(
            settings,
            environment,
            Arc::new(RecordingTransport {
                status,
                body,
                forms: Arc::clone(&forms),
            }),
            Arc::new(FixedClock),
        )
        .expect("exchange builds");
        Self {
            exchange,
            forms,
            resolver: FixedResolver { assertions },
        }
    }

    pub fn forms(&self) -> Vec<(String, Vec<(String, String)>)> {
        self.forms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn unscripted() -> SourceFailure {
    SourceFailure {
        class: "CredentialSources".to_owned(),
        status: None,
        message: "not scripted in this fixture".to_owned(),
    }
}

impl CredentialSources for ScriptedSources {
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

    fn oidc(&self, input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        self.exchange
            .token(input, &self.resolver)
            .map_err(|error| SourceFailure {
                class: "AzureOpenAIError".to_owned(),
                status: error.status_code(),
                message: error.to_string(),
            })
    }
}

struct DummyProvider;

impl CallableTokenProvider for DummyProvider {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Err(ProviderFailure {
            class: "Dummy".to_owned(),
            message: "not scripted".to_owned(),
        })
    }
}

pub fn callable_provider(identity: u64) -> CallableProvider {
    CallableProvider {
        provider: Arc::new(DummyProvider),
        identity,
    }
}

pub struct CallFixture {
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub api_version: String,
    pub dynamic_params: bool,
    pub data: Extra,
    pub model: String,
    pub timeout: Option<f64>,
    pub max_retries: i64,
    pub azure_ad_token: Option<SecretString>,
    pub provider: Option<CallableProvider>,
    pub litellm_params: LiteLlmParams,
    pub supplied: Option<StreamClient>,
}

pub fn chat_data() -> Extra {
    let mut data = Extra::new();
    let mut message = IndexMap::new();
    message.insert("role".to_owned(), Json::Text("user".to_owned()));
    message.insert("content".to_owned(), Json::Text("Hello".to_owned()));
    data.insert(
        "messages".to_owned(),
        Json::List(vec![Json::Object(message)]),
    );
    data.insert("model".to_owned(), Json::Text("deployment".to_owned()));
    data
}

impl Default for CallFixture {
    fn default() -> Self {
        Self {
            api_base: "https://unit.test".to_owned(),
            api_key: Some(SecretString::from("call-key")),
            api_version: "2024-02-01".to_owned(),
            dynamic_params: false,
            data: chat_data(),
            model: "deployment".to_owned(),
            timeout: Some(30.0),
            max_retries: 0,
            azure_ad_token: None,
            provider: None,
            litellm_params: LiteLlmParams::default(),
            supplied: None,
        }
    }
}

impl CallFixture {
    pub fn call(&self) -> StreamCall<'_> {
        StreamCall {
            api_base: &self.api_base,
            api_key: self.api_key.as_ref(),
            api_version: &self.api_version,
            dynamic_params: self.dynamic_params,
            data: &self.data,
            model: &self.model,
            timeout: self.timeout,
            max_retries: self.max_retries,
            azure_ad_token: self.azure_ad_token.as_ref(),
            azure_ad_token_provider: self.provider.as_ref(),
            litellm_params: &self.litellm_params,
            supplied: self.supplied.as_ref(),
        }
    }
}

pub const SSE_EVENTS: &str = concat!(
    "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",",
    "\"created\":1700000000,\"model\":\"deployment\",",
    "\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hello\"},",
    "\"finish_reason\":null}]}\n\n",
    "data: [DONE]\n\n"
);

pub fn sse_payload() -> Vec<Result<Vec<u8>, String>> {
    vec![Ok(SSE_EVENTS.as_bytes().to_vec())]
}

pub fn secret_headers(pairs: &[(&str, &str)]) -> SecretHeaders {
    SecretHeaders(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    )
}

pub fn raised_with(status: Option<u16>, message: String) -> Raised {
    Raised::raised(
        "ScriptedError".to_owned(),
        Reported::Present(status),
        Reported::Present(Some(message.clone())),
        message,
    )
}

#[must_use]
pub const fn json_text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub fn block_on<Fut>(future: Fut) -> Fut::Output
where
    Fut: Future,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime builds")
        .block_on(future)
}

pub fn scripted_source(
    kind: ClientKind,
    headers: &[(&str, &str)],
    payload: Vec<Result<Vec<u8>, String>>,
) -> ScriptedSource {
    let pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ScriptedSource {
        kind,
        api_key: Some(SecretString::from("client-key")),
        transport: Arc::new(ScriptedTransport {
            headers: pairs,
            payload,
            failure: None,
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        get_error: None,
        gets: Arc::new(Mutex::new(Vec::new())),
    }
}

pub fn failing_source(kind: ClientKind, failure: Raised) -> ScriptedSource {
    ScriptedSource {
        transport: Arc::new(ScriptedTransport {
            headers: Vec::new(),
            payload: Vec::new(),
            failure: Some(failure),
            requests: Arc::new(Mutex::new(Vec::new())),
        }),
        ..scripted_source(kind, &[], Vec::new())
    }
}

pub fn azure_failure(error: StreamingError) -> AzureStreamFailure {
    match error {
        StreamingError::Azure(failure) => failure,
        StreamingError::Logging(_) | StreamingError::Chunk(_) | StreamingError::Usage(_) => {
            panic!("expected an Azure failure, got {error:?}")
        }
    }
}

pub fn requests_of(source: &ScriptedSource) -> Vec<RequestRecord> {
    source
        .transport
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

pub fn gets_of(source: &ScriptedSource) -> Vec<GetRecord> {
    source
        .gets
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}
