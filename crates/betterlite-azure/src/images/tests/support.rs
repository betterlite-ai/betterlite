use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::image_generation::logging::{
    ImageGenerationLogger, PostCallRecord, PreCallRecord,
};
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime as TokioRuntime};

use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use crate::endpoints::projection::LiteLlmParams;
use crate::image_transport::request::{ImageClientConstructor, TimeoutPhases};
use crate::image_transport::{ImageClock, ImageReply, ImageRequest, ImageTransport};
use crate::images::error::ImageError;
use crate::images::generation::{GeneratedImage, Generation, Runtime, image_generation};
use crate::oidc_exchange::OidcExchangeInput;

pub(super) const API_KEY: &str = "fixture-secret-key";
pub(super) const BEARER: &str = "fixture-ad-token";
pub(super) const PROMPT: &str = "a cat wearing a hat";
pub(super) const AZURE: &str = "https://example.openai.azure.com";
pub(super) const VERSION: &str = "2025-02-01-preview";

pub(super) fn driver() -> TokioRuntime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) struct Reply {
    pub status: u16,
    pub body: &'static str,
}

pub(super) fn reply(status: u16, body: &'static str) -> Reply {
    Reply { status, body }
}

pub(super) fn ok_reply() -> Reply {
    reply(200, DALL_E_BODY)
}

// an edit's form fields: the model only when the caller sent one, plus the prompt
pub(super) fn edit_form(model: Option<&str>) -> IndexMap<String, Json> {
    let mut data = IndexMap::new();
    if let Some(name) = model {
        data.insert("model".to_owned(), Json::Text(name.to_owned()));
    }
    data.insert("prompt".to_owned(), Json::Text(PROMPT.to_owned()));
    data
}

// A reply body LiteLLM converts into an ImageResponse, with the size and quality the request asked for
pub(super) const DALL_E_BODY: &str =
    r#"{"created":1700000000,"data":[{"b64_json":"aW1hZ2U=","revised_prompt":"a cat"}]}"#;
pub(super) const MAI_BODY: &str =
    r#"{"created":1700000000,"data":[{"b64_json":"aW1hZ2U="}],"usage":{"total_tokens":7}}"#;

#[derive(Clone)]
pub(super) enum ProviderScript {
    Token(&'static str),
    Failure(&'static str, &'static str),
}

pub(super) struct FixtureProvider {
    script: ProviderScript,
}

impl CallableTokenProvider for FixtureProvider {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        match &self.script {
            ProviderScript::Token(token) => Ok(TokenReply::Text(SecretString::from(*token))),
            ProviderScript::Failure(class, message) => Err(ProviderFailure {
                class: (*class).to_owned(),
                message: (*message).to_owned(),
            }),
        }
    }
}

pub(super) struct NoSources;

fn unavailable(kind: &str) -> SourceFailure {
    SourceFailure {
        class: "ValueError".to_owned(),
        status: None,
        message: format!("no {kind} source is configured"),
    }
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("client secret"))
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("username and password"))
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("SDK refresh"))
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unavailable("OIDC"))
    }
}

pub(super) struct RecordedCall {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Json>,
}

impl RecordedCall {
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub(super) fn body_field(&self, name: &str) -> Option<&Json> {
        match self.body.as_ref()? {
            Json::Object(fields) => fields.get(name),
            _ => None,
        }
    }
}

struct FixtureState {
    replies: Mutex<Vec<Reply>>,
    calls: Mutex<Vec<RecordedCall>>,
}

#[derive(Clone)]
pub(super) struct FixtureTransport {
    state: Arc<FixtureState>,
}

impl ImageTransport for FixtureTransport {
    fn send(
        &self,
        request: ImageRequest,
    ) -> impl Future<Output = Result<ImageReply, TransportError>> + Send {
        lock(&self.state.calls).push(RecordedCall {
            method: request.method.to_string(),
            url: request.url.clone(),
            headers: request
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        value.to_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect(),
            body: (!request.body.is_empty())
                .then(|| serde_json::from_slice(&request.body).unwrap()),
        });
        let reply = lock(&self.state.replies).pop().expect("a scripted reply");
        ready(Ok(ImageReply {
            status: reply.status,
            headers: http::HeaderMap::new(),
            body: reply.body.as_bytes().to_vec(),
        }))
    }
}

pub(super) struct FixtureClock;

impl ImageClock for FixtureClock {
    fn now(&self) -> Duration {
        Duration::from_secs(0)
    }

    fn pause(&self, _seconds: i64) -> impl Future<Output = ()> + Send {
        ready(())
    }
}

pub(super) struct FixtureConstructor {
    transport: FixtureTransport,
    constructed: Arc<Mutex<Vec<TimeoutPhases>>>,
}

impl ImageClientConstructor for FixtureConstructor {
    type Client = FixtureTransport;

    fn construct(&self, timeout: TimeoutPhases) -> FixtureTransport {
        lock(&self.constructed).push(timeout);
        self.transport.clone()
    }
}

#[derive(Default)]
pub(super) struct RecordingLogger {
    pub pre: Vec<PreCallRecord>,
    pub post: Vec<PostCallRecord>,
}

impl ImageGenerationLogger for RecordingLogger {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        self.pre.push(record);
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        self.post.push(record);
    }
}

#[derive(Default)]
pub(super) struct Scenario {
    pub model: Option<String>,
    pub base_model: Option<String>,
    pub prompt: String,
    pub optional: String,
    pub headers: Vec<(String, String)>,
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub azure_ad_token: Option<String>,
    pub provider: Option<ProviderScript>,
    pub params: String,
    pub delegate_async: bool,
    pub timeout: Option<f64>,
}

impl Scenario {
    pub(super) fn new() -> Self {
        Self {
            prompt: PROMPT.to_owned(),
            ..Self::default()
        }
    }

    fn optional_params(&self) -> IndexMap<String, Json> {
        if self.optional.is_empty() {
            return IndexMap::new();
        }
        serde_json::from_str(&self.optional).unwrap()
    }

    fn litellm_params(&self) -> LiteLlmParams {
        if self.params.is_empty() {
            return LiteLlmParams::default();
        }
        serde_json::from_str(&self.params).unwrap()
    }
}

// the request the Azure image call always receives from its caller
pub(super) fn scenario(model: &str) -> Scenario {
    Scenario {
        model: Some(model.to_owned()),
        api_key: Some(API_KEY.to_owned()),
        api_base: Some(AZURE.to_owned()),
        ..Scenario::new()
    }
}

pub(super) struct Outcome {
    pub result: Result<GeneratedImage, ImageError>,
    pub calls: Vec<RecordedCall>,
    pub constructed: usize,
    pub logger: RecordingLogger,
}

impl Outcome {
    pub(super) fn success(&self) -> &GeneratedImage {
        self.result.as_ref().expect("the call succeeds")
    }

    pub(super) fn failure(&self) -> &ImageError {
        self.result.as_ref().expect_err("the call fails")
    }

    pub(super) fn url(&self) -> &str {
        &self.calls[0].url
    }

    pub(super) fn sent_headers(&self) -> &[(String, String)] {
        &self.calls[0].headers
    }

    pub(super) fn pre_headers(&self) -> &IndexMap<String, Json> {
        &self.logger.pre[0].headers
    }

    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.calls[0].header(name)
    }
}

pub(super) struct Harness {
    pub config: Config,
    pub environment: BTreeMap<String, String>,
    pub replies: Vec<Reply>,
}

impl Harness {
    pub(super) fn new(environment: &[(&str, &str)]) -> Self {
        let environment: BTreeMap<String, String> = environment
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        let config = ConfigLoader {
            lookup: environment.clone(),
        }
        .load("")
        .unwrap();
        Self {
            config,
            environment,
            replies: Vec::new(),
        }
    }

    pub(super) fn with_reply(mut self, reply: Reply) -> Self {
        self.replies.push(reply);
        self
    }

    pub(super) fn run(&mut self, scenario: &Scenario) -> Outcome {
        let constructor = FixtureConstructor {
            transport: FixtureTransport {
                state: Arc::new(FixtureState {
                    replies: Mutex::new(std::mem::take(&mut self.replies)),
                    calls: Mutex::default(),
                }),
            },
            constructed: Arc::default(),
        };
        let clock = FixtureClock;
        let sources = NoSources;
        let mut logger = RecordingLogger::default();
        let api_key = scenario.api_key.as_deref().map(SecretString::from);
        let azure_ad_token = scenario.azure_ad_token.as_deref().map(SecretString::from);
        let provider = scenario
            .provider
            .clone()
            .map(|script| FixtureProvider { script });
        let call = Generation {
            model: scenario.model.as_deref(),
            prompt: &scenario.prompt,
            timeout: scenario.timeout,
            optional_params: scenario.optional_params(),
            headers: scenario.headers.clone(),
            api_key: api_key.as_ref(),
            api_base: scenario.api_base.as_deref(),
            api_version: scenario.api_version.as_deref(),
            azure_ad_token: azure_ad_token.as_ref(),
            azure_ad_token_provider: provider
                .map(|provider| Arc::new(provider) as Arc<dyn CallableTokenProvider>),
            base_model: scenario.base_model.as_deref(),
            params: scenario.litellm_params(),
            delegate_async: scenario.delegate_async,
            now_seconds: 1_700_000_000,
        };
        let endpoints = &self.config.sections.azure_endpoints;
        let polling_timeout = self
            .config
            .sections
            .azure_image_transport
            .operation_polling_timeout;
        let refresh_enabled = self
            .config
            .sections
            .azure_credentials
            .enable_azure_ad_token_refresh;
        let mut state = Runtime {
            endpoints,
            environment: &self.environment,
            sources: &sources,
            refresh_enabled,
            polling_timeout,
            constructor: &constructor,
            clock: &clock,
            logger: &mut logger,
        };
        let result = driver().block_on(image_generation(call, &mut state));
        Outcome {
            result,
            calls: lock(&constructor.transport.state.calls).clone(),
            constructed: lock(&constructor.constructed).len(),
            logger,
        }
    }
}
