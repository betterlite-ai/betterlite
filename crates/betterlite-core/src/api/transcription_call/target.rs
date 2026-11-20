use std::time::Duration;

use betterlite_context::media_config_selection::audio_transcription::AudioTranscriptionConfigTag;
use betterlite_context::non_default_params::Params;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::transcription::client::SharedSession;
use betterlite_openai::transcription::config::TranscriptionFamily;
use indexmap::IndexMap;
use secrecy::SecretString;
use uuid::Uuid;

use super::call::SuppliedClientKind;
use super::decisions_json;

// every branch's arguments; each provider target keeps the ones its handler reads
pub struct TranscriptionInput<'call> {
    pub model: &'call str,
    pub input: &'call str,
    pub file: &'call ProcessedAudioFile,
    pub optional_params: Params,
    pub litellm_params: Params,
    pub atranscription: bool,
    pub timeout: Duration,
    pub max_retries: u32,
    pub api_base: Option<&'call str>,
    pub api_key: Option<&'call SecretString>,
    pub client: Option<SuppliedClientKind>,
    pub shared_session: Option<&'call SharedSession>,
    pub boundary: Uuid,
}

impl TranscriptionInput<'_> {
    // the optional params as B515's request takes them, in B515's own JSON shape
    #[must_use]
    pub fn optional_params_map(&self) -> IndexMap<String, Json> {
        self.optional_params
            .iter()
            .map(|(name, value)| (name.clone(), decisions_json(value)))
            .collect()
    }

    // the optional params as B2453's request takes them, in serde_json's object shape
    #[must_use]
    pub fn optional_params_object(&self) -> Object {
        object_from_params(&self.optional_params)
    }

    // litellm_params as get_litellm_params hands them to B2453
    #[must_use]
    pub fn litellm_params_object(&self) -> Object {
        object_from_params(&self.litellm_params)
    }
}

// a closed JSON tree converts to serde_json's object without naming its value type
fn object_from_params(params: &Params) -> Object {
    let mut object = Object::new();
    for (name, value) in params {
        let converted = serde_json::to_value(value).unwrap_or_default();
        object.insert(name.clone(), converted);
    }
    object
}

// openai_audio_transcriptions.audio_transcriptions over B515, for openai and its compatibles
pub struct OpenAiTranscription<'call> {
    pub input: TranscriptionInput<'call>,
    pub organization: Option<&'call str>,
    pub family: Option<TranscriptionFamily>,
}

// azure_audio_transcriptions.audio_transcriptions over B539, when no provider config was selected
pub struct AzureTranscription<'call> {
    pub input: TranscriptionInput<'call>,
    pub api_version: Option<&'call str>,
    pub azure_ad_token: Option<&'call SecretString>,
}

// the injected operations: Riva, Soniox, Bedrock, the generic native handler and Watsonx
pub struct InjectedTranscription<'call> {
    pub input: TranscriptionInput<'call>,
    // Some({}) for the generic handler, the caller's extra_headers for Soniox and Bedrock
    pub headers: Option<&'call Headers>,
    pub config: Option<AudioTranscriptionConfigTag>,
}

pub enum TranscriptionTarget<'call> {
    OpenAi(OpenAiTranscription<'call>),
    Azure(AzureTranscription<'call>),
    NvidiaRiva(InjectedTranscription<'call>),
    Soniox(InjectedTranscription<'call>),
    Bedrock(InjectedTranscription<'call>),
    Native(InjectedTranscription<'call>),
    Watsonx(InjectedTranscription<'call>),
}

// litellm_params.timeout is the signature's 600 unless the caller named one
pub(super) fn timeout_seconds(timeout: Option<f64>) -> f64 {
    timeout.unwrap_or(super::DEFAULT_TIMEOUT_SECONDS)
}

// the OpenAI SDK compares its attempt count against max_retries, so a negative value behaves as zero
pub(super) fn retry_count(max_retries: Option<i64>) -> u32 {
    max_retries.map_or(super::DEFAULT_MAX_RETRIES, |retries| {
        u32::try_from(retries.max(0)).unwrap_or(u32::MAX)
    })
}
