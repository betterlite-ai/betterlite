use std::collections::BTreeMap;
use std::future::Future;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use betterlite_config::sections::regional_host_keys::Settings as SectionSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::speech_provider_contract::{SpeechVoice, TextToSpeechRequestData};
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::minimax_speech::params::{MappedOptions, VoiceInput, map_openai_params};
use betterlite_openai::minimax_speech::reply::{SpeechReply, decode_speech_reply};
use betterlite_openai::minimax_speech::request::{
    TTS_BASE_URL, get_complete_url, speech_request_body, validate_environment,
};
use betterlite_openai::openai_base_config::Object as SpeechObject;
use betterlite_openai::speech::result::SpeechResponse;
use betterlite_prompts::arguments::json::{Json, Object};
use futures_util::Stream as _;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::api::speech_http::SpeechCall;
use crate::api::speech_http::SpeechParams;
use crate::api::speech_provider_contract::error::SpeechContractError;
use crate::api::speech_provider_contract::{
    ClassAttribute, SpeechHeaders, SpeechMappingOptions, SpeechRequestInput, TextToSpeechConfig,
};

use super::context::first_secret;
use super::error::DispatchError;
use super::logging;

const MINIMAX_MISSING_KEY: &str = "MiniMax API key is required. Set MINIMAX_API_KEY environment variable or pass api_key parameter.";

// the speech arm's request, before voice and litellm_params take their LiteLLM shape
pub struct SpeechContext<'request> {
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub extra_headers: Option<&'request SpeechHeaders>,
    pub input: &'request str,
    pub litellm_params: &'request Object,
    pub model: &'request str,
    pub optional_params: &'request Object,
    pub ssl_verify: Option<bool>,
    pub timeout: Option<Duration>,
    pub voice: Option<SpeechVoice>,
    pub settings: &'request SectionSettings,
}

// the shared HTTP handler B2452 exposes, injected so the arm stays free of a transport
pub trait SpeechHandler {
    type Reply;

    fn synthesize(
        &self,
        call: &SpeechCall<'_>,
        config: &dyn TextToSpeechConfig,
    ) -> impl Future<Output = Result<Self::Reply, DispatchError>> + Send;
}

// speech()'s minimax branch: the selected config or B8174's, the caller's base and key into litellm_params only when set, then the handler
pub async fn minimax<H: SpeechHandler>(
    provider_config: Option<&dyn TextToSpeechConfig>,
    context: &SpeechContext<'_>,
    handler: &H,
) -> Result<H::Reply, DispatchError> {
    let key = first_secret([
        context.api_key,
        context.settings.api_key.as_ref(),
        context.settings.environment.minimax_api_key.as_ref(),
    ])
    .ok_or(DispatchError::MissingKey {
        message: MINIMAX_MISSING_KEY,
    })?;
    let selected = MinimaxSpeechConfig {
        env_base: context.settings.environment.minimax_api_base.as_deref(),
    };
    // speech() maps voice and options before the arm when a config is selected; with none the arm uses B8174's and maps here
    let (voice, optional_params) = match provider_config {
        Some(_given) => (
            context.voice.clone(),
            logging::convert(context.optional_params)?,
        ),
        None => mapped(context.voice.clone(), context.optional_params)?,
    };
    let mut litellm_params: SpeechObject = logging::convert(context.litellm_params)?;
    if let Some(api_base) = context.api_base {
        litellm_params.insert(String::from("api_base"), Value::String(api_base.to_owned()));
    }
    if let Some(api_key) = context.api_key {
        litellm_params.insert(
            String::from("api_key"),
            Value::String(api_key.expose_secret().to_owned()),
        );
    }
    let params = SpeechParams {
        api_key: Some(key),
        api_base: context.api_base.map(str::to_owned),
        ssl_verify: context.ssl_verify,
    };
    let voice_text = voice_text(voice.as_ref());
    let call = SpeechCall {
        model: context.model,
        input: context.input,
        voice: voice_text.as_deref(),
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        extra_headers: context.extra_headers,
        params: &params,
        timeout: context.timeout,
    };
    let config: &dyn TextToSpeechConfig = provider_config.unwrap_or(&selected);
    handler.synthesize(&call, config).await
}

fn mapped(
    voice: Option<SpeechVoice>,
    optional_params: &Object,
) -> Result<(Option<SpeechVoice>, SpeechObject), DispatchError> {
    let given = voice.as_ref().map(voice_input);
    let options = map_minimax(given.as_ref(), &logging::convert(optional_params)?)?;
    Ok((Some(SpeechVoice::Name(options.voice)), options.params))
}

fn map_minimax(
    voice: Option<&VoiceInput<'_>>,
    optional_params: &SpeechObject,
) -> Result<MappedOptions, DispatchError> {
    map_openai_params(voice, optional_params)
        .map_err(|source| DispatchError::Speech(Box::new(source)))
}

fn voice_input(voice: &SpeechVoice) -> VoiceInput<'_> {
    match voice {
        SpeechVoice::Name(name) => VoiceInput::Text(name),
        SpeechVoice::Options(fields) => VoiceInput::Fields(fields),
    }
}

// the arm's own dict step: the first non-empty voice_id, id or name, while a string voice passes as it is
fn voice_text(voice: Option<&SpeechVoice>) -> Option<String> {
    match voice {
        Some(SpeechVoice::Name(name)) => Some(name.clone()),
        Some(SpeechVoice::Options(fields)) => ["voice_id", "id", "name"].iter().find_map(|name| {
            fields
                .get(*name)
                .and_then(|value| value.as_str())
                .filter(|text| !text.trim().is_empty())
                .map(str::to_owned)
        }),
        None => None,
    }
}

// MiniMaxTextToSpeechConfig over B8174's transforms; the caller resolves the key before the handler runs
pub struct MinimaxSpeechConfig<'settings> {
    pub env_base: Option<&'settings str>,
}

impl TextToSpeechConfig for MinimaxSpeechConfig<'_> {
    // only the base constant is exported by B8174; the endpoint and the two mappings stay private there
    fn class_attributes(&self) -> IndexMap<String, ClassAttribute> {
        IndexMap::from([(
            String::from("TTS_BASE_URL"),
            ClassAttribute::Value(Json::Text(String::from(TTS_BASE_URL))),
        )])
    }

    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        betterlite_openai::minimax_speech::params::SUPPORTED_OPENAI_PARAMS
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    fn map_openai_params(
        &self,
        _model: &str,
        optional_params: SpeechObject,
        options: &SpeechMappingOptions,
    ) -> (Option<String>, SpeechObject) {
        let voice = options.voice.as_ref().map(voice_input);
        match map_openai_params(voice.as_ref(), &optional_params) {
            Ok(mapped) => (Some(mapped.voice), mapped.params),
            Err(_error) => (None, optional_params),
        }
    }

    fn validate_environment(
        &self,
        headers: &SpeechHeaders,
        _model: &str,
        api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> SpeechHeaders {
        let caller: BTreeMap<String, String> = headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        match validate_environment(&caller, api_key, None, None) {
            Ok(merged) => merged.into_iter().collect(),
            Err(_error) => headers.clone(),
        }
    }

    fn complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &SpeechObject,
    ) -> Result<String, SpeechContractError> {
        Ok(get_complete_url(api_base, self.env_base))
    }

    fn speech_request(&self, request: &SpeechRequestInput<'_>) -> TextToSpeechRequestData {
        TextToSpeechRequestData {
            dict_body: Some(speech_request_body(
                request.model,
                request.input,
                request.voice.unwrap_or_default(),
                request.optional_params,
            )),
            ssml_body: None,
            headers: Some(IndexMap::from([(
                String::from("Content-Type"),
                String::from("application/json"),
            )])),
        }
    }

    // LiteLLM raises on a reply it cannot decode; the transform here has no error channel, so the raw reply passes (docs/deviations/B8312.yaml)
    fn speech_response(&self, _model: &str, raw_response: UpstreamResponse) -> BinaryContent {
        let reply = SpeechReply {
            status: raw_response.status,
            headers: &raw_response.headers,
            body: &raw_response.body,
        };
        match decode_speech_reply(&reply) {
            Ok(decoded) => {
                let status = decoded.status;
                let headers = decoded.headers.clone();
                let body = decoded_audio(decoded);
                BinaryContent {
                    status,
                    headers,
                    body,
                }
            }
            Err(_error) => BinaryContent::from_response(raw_response),
        }
    }
}

// the decoded audio is the only item in the one-chunk stream B8174's decoder builds, so one poll drains it
fn decoded_audio(response: SpeechResponse) -> Vec<u8> {
    let mut body = response.body;
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    match body.as_mut().poll_next(&mut context) {
        Poll::Ready(Some(Ok(bytes))) => bytes,
        Poll::Ready(Some(Err(_error)) | None) | Poll::Pending => Vec::new(),
    }
}
