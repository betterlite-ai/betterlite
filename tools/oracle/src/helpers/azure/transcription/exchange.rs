mod view;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::endpoints::get_client::ClientCache;
use betterlite_azure::endpoints::projection::{
    ClientIdentity, ClientKind, LiteLlmParams, SessionKind, SuppliedClient,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_azure::transcription::call::client::ClientWiring;
use betterlite_azure::transcription::call::logging::{
    AzureTranscriptionLogger, AzureTranscriptionPostCall, AzureTranscriptionPreCall,
};
use betterlite_azure::transcription::call::{
    AzureTranscriptionCall, audio_transcriptions, finish_transcription,
};
use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use http::HeaderMap;
use http::header::{CONTENT_LENGTH, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use uuid::Uuid;

use self::view::{error_view, request_view, result_view};
use super::{Probe, Reply};
use crate::helpers::{HelperError, OrHelperError as _};

const FILE_NAME: &str = "a.wav";
const AUDIO: &[u8] = b"abc";

struct Fixture {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Fixture {
    fn new(reply: &Reply) -> Result<Self, Box<HelperError>> {
        let mut headers = HeaderMap::new();
        for (name, value) in &reply.headers {
            headers.append(
                HeaderName::from_bytes(name.as_bytes()).or_config_error()?,
                HeaderValue::from_str(value).or_config_error()?,
            );
        }
        // httpx fills in Content-Length for a non-empty body, and the async branch forwards every response header
        if !reply.body.is_empty() && !headers.contains_key(CONTENT_LENGTH) {
            headers.insert(
                CONTENT_LENGTH,
                HeaderValue::try_from(reply.body.len().to_string()).or_config_error()?,
            );
        }
        Ok(Self {
            status: reply.status,
            headers,
            body: reply.body.clone().into_bytes(),
        })
    }
}

// no credential source resolves anything: the call's own api_key is the only credential
#[derive(Default)]
struct NoSources;

fn no_source() -> SourceFailure {
    SourceFailure {
        class: String::from("RuntimeError"),
        status: None,
        message: String::from("no credential source in the oracle fixture"),
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

#[derive(Default)]
struct Cache {
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

#[derive(Default)]
struct Recorder {
    pre: Value,
    post: Value,
}

impl AzureTranscriptionLogger for Recorder {
    fn pre_call(&mut self, record: AzureTranscriptionPreCall<'_>) -> Result<(), PrePostError> {
        self.pre = json!({
            "input_names_file": record.input.starts_with("audio_file_"),
            "authorization": record.authorization,
            "api_base": record.api_base,
            "data": record.data,
        });
        Ok(())
    }

    fn post_call(&mut self, record: AzureTranscriptionPostCall<'_>) {
        let (original, error_text) = match &record.original_response {
            Json::Object(_) => (record.original_response.clone(), false),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                (Value::Null, true)
            }
        };
        self.post = json!({
            "input_names_file": record.input.contains(FILE_NAME),
            "authorization": record.authorization,
            "api_base": record.api_base,
            "original_response": original,
            "data": record.data,
            "error_text": error_text,
        });
    }
}

pub(super) fn exchange(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let file = processed_file();
    let call = call_for(probe, &file);
    let settings = Settings::default();
    let environment = BTreeMap::new();
    let sources = NoSources;
    let mut cache = Cache::default();
    let mut refresh = RefreshProviderCache::new(&sources);
    let mut wiring = ClientWiring {
        settings: &settings,
        environment: &environment,
        sources: &sources,
        cache: &mut cache,
        refresh: &mut refresh,
    };
    let mut recorder = Recorder::default();
    let outcome = run_exchange(&call, probe, &mut wiring, &mut recorder)?;
    Ok(json!({
        "request": outcome.request,
        "result": outcome.result,
        "error": outcome.error,
        "pre_call": recorder.pre,
        "post_call": recorder.post,
    }))
}

fn processed_file() -> ProcessedAudioFile {
    ProcessedAudioFile {
        file_content: AUDIO.to_vec(),
        filename: String::from(FILE_NAME),
        content_type: String::from("audio/wav"),
    }
}

fn call_for<'data>(
    probe: &'data Probe,
    file: &'data ProcessedAudioFile,
) -> AzureTranscriptionCall<'data> {
    AzureTranscriptionCall {
        input: String::from(FILE_NAME),
        file,
        boundary: Uuid::from_u128(539),
        model: &probe.model,
        optional_params: probe.optional_params.clone(),
        atranscription: probe.atranscription,
        api_version: probe.api_version.clone(),
        api_key: probe.api_key.clone().map(SecretString::from),
        api_base: probe.api_base.clone(),
        azure_ad_token: probe.azure_ad_token.clone().map(SecretString::from),
        max_retries: probe.max_retries,
        timeout: Duration::from_secs(30),
        params: LiteLlmParams::default(),
        provider: None,
        supplied: probe.client_kind.as_deref().map(|kind| SuppliedClient {
            kind: if kind == "openai" {
                ClientKind::OpenAi
            } else {
                ClientKind::AzureOpenAi
            },
            custom_query: None,
        }),
        session: SessionKind::Dedicated,
        refresh_enabled: false,
    }
}

struct Outcome {
    request: Value,
    result: Value,
    error: Value,
}

fn run_exchange(
    call: &AzureTranscriptionCall<'_>,
    probe: &Probe,
    wiring: &mut ClientWiring<'_, '_>,
    recorder: &mut Recorder,
) -> Result<Outcome, Box<HelperError>> {
    match audio_transcriptions(call, wiring, recorder) {
        Err(failure) => Ok(Outcome {
            request: Value::Null,
            result: Value::Null,
            error: error_view(&failure),
        }),
        Ok((outbound, pending)) => {
            let view = request_view(&outbound);
            let fixture = Fixture::new(&probe.reply)?;
            let sent = UpstreamResponse {
                status: fixture.status,
                headers: fixture.headers,
                body: fixture.body,
            };
            match finish_transcription(pending, Ok(sent), recorder) {
                Ok(reply) => Ok(Outcome {
                    request: view,
                    result: result_view(&reply)?,
                    error: Value::Null,
                }),
                Err(failure) => Ok(Outcome {
                    request: view,
                    result: Value::Null,
                    error: error_view(&failure),
                }),
            }
        }
    }
}
