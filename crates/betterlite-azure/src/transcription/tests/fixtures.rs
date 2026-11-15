#![cfg(test)]

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::transcription::reply::TranscriptionReply;
use http::header::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;
use uuid::Uuid;

use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::{CallableTokenProvider, CredentialSources, SourceFailure};
use crate::endpoints::get_client::ClientCache;
use crate::endpoints::projection::{ClientIdentity, ClientKind, LiteLlmParams, SessionKind};
use crate::oidc_exchange::OidcExchangeInput;
use crate::transcription::call::client::ClientWiring;
use crate::transcription::call::logging::{
    AzureTranscriptionLogger, AzureTranscriptionPostCall, AzureTranscriptionPreCall,
};
use crate::transcription::call::{
    AzureTranscriptionCall, AzureTranscriptionError, Pending, audio_transcriptions,
    finish_transcription,
};

pub(crate) const API_BASE: &str = "https://example.openai.azure.com";
pub(crate) const API_KEY: &str = "sk-fixture";
pub(crate) const API_VERSION: &str = "2024-06-01";
pub(crate) const BOUNDARY: u128 = 1;
pub(crate) const INPUT: &str = "a.wav";

pub(crate) fn audio_file() -> ProcessedAudioFile {
    ProcessedAudioFile {
        file_content: b"abc".to_vec(),
        filename: String::from(INPUT),
        content_type: String::from("audio/wav"),
    }
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn params(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn azure_call<'file>(file: &'file ProcessedAudioFile) -> AzureTranscriptionCall<'file> {
    AzureTranscriptionCall {
        input: String::from(INPUT),
        file,
        boundary: Uuid::from_u128(BOUNDARY),
        model: "whisper",
        optional_params: IndexMap::new(),
        atranscription: false,
        api_version: Some(API_VERSION.to_owned()),
        api_key: Some(SecretString::from(API_KEY)),
        api_base: Some(API_BASE.to_owned()),
        azure_ad_token: None,
        max_retries: None,
        timeout: Duration::from_secs(30),
        params: LiteLlmParams::default(),
        provider: None,
        supplied: None,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
    }
}

#[derive(Default)]
pub(crate) struct Cache {
    entries: HashMap<ClientIdentity, ClientKind>,
}

impl ClientCache for Cache {
    fn get(&self, identity: &ClientIdentity) -> Option<ClientKind> {
        self.entries.get(identity).copied()
    }

    fn set(&mut self, identity: ClientIdentity, kind: ClientKind, _owned: bool) {
        self.entries.insert(identity, kind);
    }
}

impl Cache {
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

// no credential source resolves anything: an owned call uses the values it carries itself
#[derive(Default)]
pub(crate) struct NoSources;

fn no_source() -> SourceFailure {
    SourceFailure {
        class: String::from("RuntimeError"),
        status: None,
        message: String::from("no credential source in this fixture"),
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

pub(crate) struct PreCallRecord {
    pub(crate) input: String,
    pub(crate) authorization: String,
    pub(crate) api_base: String,
    pub(crate) data: IndexMap<String, Json>,
}

pub(crate) struct PostCallRecord {
    pub(crate) input: String,
    pub(crate) authorization: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) original_response: Json,
    pub(crate) data: Option<IndexMap<String, Json>>,
}

#[derive(Default)]
pub(crate) struct Recorder {
    pub(crate) pre: Option<PreCallRecord>,
    pub(crate) posts: Vec<PostCallRecord>,
}

impl AzureTranscriptionLogger for Recorder {
    fn pre_call(&mut self, record: AzureTranscriptionPreCall<'_>) -> Result<(), PrePostError> {
        self.pre = Some(PreCallRecord {
            input: record.input.to_owned(),
            authorization: record.authorization.to_owned(),
            api_base: record.api_base.to_owned(),
            data: record.data,
        });
        Ok(())
    }

    fn post_call(&mut self, record: AzureTranscriptionPostCall<'_>) {
        self.posts.push(PostCallRecord {
            input: record.input.to_owned(),
            authorization: record.authorization.map(str::to_owned),
            api_base: record.api_base.map(str::to_owned),
            original_response: record.original_response,
            data: record.data,
        });
    }
}

pub(crate) struct Inputs {
    pub(crate) settings: Settings,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) sources: NoSources,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            environment: BTreeMap::new(),
            sources: NoSources,
        }
    }
}

pub(crate) fn start(
    inputs: &Inputs,
    cache: &mut Cache,
    call: &AzureTranscriptionCall<'_>,
    logger: &mut Recorder,
) -> Result<(OutboundRequest, Pending), AzureTranscriptionError> {
    let mut refresh = RefreshProviderCache::new(&inputs.sources);
    let mut wiring = ClientWiring {
        settings: &inputs.settings,
        environment: &inputs.environment,
        sources: &inputs.sources,
        cache,
        refresh: &mut refresh,
    };
    audio_transcriptions(call, &mut wiring, logger)
}

pub(crate) fn response(status: u16, content_type: &str, body: &str) -> UpstreamResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        HeaderValue::try_from(content_type).expect("the fixture content type is a header value"),
    );
    headers.insert("x-request-id", HeaderValue::from_static("req-1"));
    UpstreamResponse {
        status,
        headers,
        body: body.as_bytes().to_vec(),
    }
}

pub(crate) fn finish(
    pending: Pending,
    sent: UpstreamResponse,
    logger: &mut Recorder,
) -> Result<TranscriptionReply, AzureTranscriptionError> {
    finish_transcription(pending, Ok(sent), logger)
}

fn parts_of(request: &OutboundRequest) -> &[MultipartPart] {
    let RequestBody::Multipart { parts, .. } = &request.body else {
        panic!("the transcription request is multipart");
    };
    parts
}

pub(crate) fn part<'request>(request: &'request OutboundRequest, name: &str) -> &'request [u8] {
    parts_of(request)
        .iter()
        .find(|part| part.name == name)
        .map(|found| found.data.as_slice())
        .unwrap_or_default()
}

pub(crate) fn part_names(request: &OutboundRequest) -> Vec<&str> {
    parts_of(request)
        .iter()
        .map(|part| part.name.as_str())
        .collect()
}

pub(crate) fn header<'request>(
    request: &'request OutboundRequest,
    name: &str,
) -> Option<&'request str> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
}
