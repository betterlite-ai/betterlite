use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use http::header::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use crate::endpoints::get_client::ClientCache;
use crate::endpoints::projection::{
    CallableProvider, ClientIdentity, ClientKind, LiteLlmParams, SessionKind,
};
use crate::oidc_exchange::OidcExchangeInput;
use crate::speech::binary::{AudioSpeechCall, audio_speech};
use crate::speech::client::ClientWiring;
use crate::speech::dispatch::{HandlerFailure, HandlerRequest, TextToSpeechHandler};
use crate::speech::error::AzureSpeechError;
use crate::speech::logging::{SpeechLogger, SpeechPreCall};

pub(crate) const API_BASE: &str = "https://example.openai.azure.com";
pub(crate) const API_KEY: &str = "sk-azure-fixture";
pub(crate) const API_VERSION: &str = "2024-06-01";
pub(crate) const SECRET: &str = "sk-azure-secret-fixture";

pub(crate) fn json(text: &str) -> Json {
    match serde_json::from_str::<Json>(text) {
        Ok(value) => value,
        Err(error) => panic!("the expected JSON parses: {error}"),
    }
}

pub(crate) fn pairs(entries: &[(&str, Json)]) -> IndexMap<String, Json> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn call<'data>(
    model: &'data str,
    input: &'data str,
    voice: &'data str,
    optional_params: IndexMap<String, Json>,
) -> AudioSpeechCall<'data> {
    AudioSpeechCall {
        model,
        input,
        voice,
        optional_params,
        aspeech: false,
        api_key: Some(SecretString::from(API_KEY)),
        api_base: Some(API_BASE.to_owned()),
        api_version: Some(API_VERSION.to_owned()),
        params: LiteLlmParams::default(),
        provider: None,
        supplied: None,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
    }
}

struct FixedToken(TokenReply);

impl CallableTokenProvider for FixedToken {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Ok(self.0.clone())
    }
}

// the callable audio_speech hands the SDK, answering with one fixed reply
pub(crate) fn provider(reply: TokenReply) -> CallableProvider {
    CallableProvider {
        provider: Arc::new(FixedToken(reply)),
        identity: 7,
    }
}

#[derive(Default)]
pub(crate) struct Cache {
    pub(crate) entries: Vec<(ClientIdentity, ClientKind)>,
}

impl ClientCache for Cache {
    fn get(&self, identity: &ClientIdentity) -> Option<ClientKind> {
        self.entries
            .iter()
            .find(|(known, _)| known == identity)
            .map(|(_, kind)| *kind)
    }

    fn set(&mut self, identity: ClientIdentity, kind: ClientKind, _owned: bool) {
        self.entries.push((identity, kind));
    }
}

// no credential source resolves anything: an owned call uses the values it carries itself
#[derive(Default)]
pub(crate) struct NoSources;

fn no_source() -> SourceFailure {
    SourceFailure {
        class: String::from("RuntimeError"),
        status: None,
        message: String::from("the speech fixture has no credential source"),
    }
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(no_source())
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(no_source())
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(no_source())
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(no_source())
    }
}

pub(crate) struct PreCall {
    pub(crate) input: String,
    pub(crate) api_base: String,
    pub(crate) data: IndexMap<String, Json>,
}

#[derive(Default)]
pub(crate) struct Recorder {
    pub(crate) pre: Option<PreCall>,
}

impl SpeechLogger for Recorder {
    fn pre_call(&mut self, record: SpeechPreCall<'_>) -> Result<(), PrePostError> {
        self.pre = Some(PreCall {
            input: record.input.to_owned(),
            api_base: record.api_base.to_owned(),
            data: record.data,
        });
        Ok(())
    }
}

pub(crate) struct Inputs {
    pub(crate) settings: Settings,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) sources: NoSources,
    pub(crate) cache: Cache,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            environment: BTreeMap::new(),
            sources: NoSources,
            cache: Cache::default(),
        }
    }
}

pub(crate) fn start(
    inputs: &mut Inputs,
    call: &AudioSpeechCall<'_>,
    logger: &mut Recorder,
) -> Result<OutboundRequest, AzureSpeechError> {
    let mut refresh = RefreshProviderCache::new(&inputs.sources);
    let mut wiring = ClientWiring {
        settings: &inputs.settings,
        environment: &inputs.environment,
        sources: &inputs.sources,
        cache: &mut inputs.cache,
        refresh: &mut refresh,
    };
    audio_speech(call, &mut wiring, logger)
}

pub(crate) fn response(
    status: u16,
    headers: &[(&'static str, &'static str)],
    body: &[u8],
) -> UpstreamResponse {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.insert(*name, HeaderValue::from_static(value));
    }
    UpstreamResponse {
        status,
        headers: map,
        body: body.to_vec(),
    }
}

pub(crate) fn header<'request>(
    request: &'request OutboundRequest,
    name: &str,
) -> Option<&'request str> {
    let value = request.headers.get(name)?;
    value.to_str().ok()
}

pub(crate) fn body(request: &OutboundRequest) -> Json {
    let RequestBody::Json(bytes) = &request.body else {
        panic!("the speech request body is JSON");
    };
    serde_json::from_slice(bytes).expect("the speech body parses")
}

#[derive(Clone)]
pub(crate) struct HandlerRecord {
    pub(crate) model: String,
    pub(crate) input: String,
    pub(crate) voice: Option<String>,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) provider: String,
    pub(crate) is_async: bool,
}

#[derive(Default)]
pub(crate) struct RecordingHandler {
    pub(crate) requests: Vec<HandlerRecord>,
}

impl TextToSpeechHandler for RecordingHandler {
    type Output = ();

    fn text_to_speech(
        &mut self,
        request: HandlerRequest<'_>,
    ) -> impl Future<Output = Result<(), HandlerFailure>> + Send {
        self.requests.push(HandlerRecord {
            model: request.model.to_owned(),
            input: request.input.to_owned(),
            voice: request.voice.map(str::to_owned),
            optional_params: request.optional_params.clone(),
            litellm_params: request.litellm_params.clone(),
            extra_headers: request.extra_headers.cloned(),
            timeout: request.timeout,
            api_key: request.api_key.map(str::to_owned),
            api_base: request.api_base.map(str::to_owned),
            provider: request.provider.to_owned(),
            is_async: request.is_async,
        });
        ready(Ok(()))
    }
}
