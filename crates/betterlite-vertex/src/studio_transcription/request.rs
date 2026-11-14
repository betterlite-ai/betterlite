use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sections::studio_identity::Settings;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::api::audio_file_input::language::normalize_language_to_bcp47;
use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, AudioTranscriptionRequestData,
};
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;

use crate::studio_interactions::credentials as studio;
use crate::studio_interactions::credentials::{RequestHeaders, StudioParams, resolve_api_key};
use crate::studio_transcription::SUPPORTED_PARAMS;
use crate::studio_transcription::error::StudioTranscriptionError;
use crate::studio_transcription::types::{
    AudioInput, AudioInputType, DiarizationMode, GenerationConfig, InteractionRequest,
    TranscriptionConfig, TranscriptionGranularity, TranscriptionModeType, VerbatimMode,
};

const MODEL_PREFIX: &str = "gemini/";
const WORD_GRANULARITY: &str = "word";
const SUBTITLE_RESPONSE_FORMATS: [&str; 2] = ["srt", "vtt"];

// LiteLLM lays the caller's optional params first and overwrites them with the supported non-defaults
#[must_use]
pub fn map_openai_params(non_default: &Object, optional: &Object) -> Object {
    let mut mapped = optional.clone();
    for name in SUPPORTED_PARAMS {
        if let Some(value) = non_default.get(name) {
            mapped.insert(name.to_owned(), value.clone());
        }
    }
    mapped
}

pub fn validate_environment(
    caller: &IndexMap<String, String>,
    params: &StudioParams,
    settings: &Settings,
) -> Result<RequestHeaders, StudioTranscriptionError> {
    if resolve_api_key(params, settings).is_none() {
        return Err(StudioTranscriptionError::MissingApiKey);
    }
    Ok(studio::validate_environment(caller, Some(params), settings))
}

fn wants_word_timestamps(optional: &Object) -> bool {
    let word_requested = optional
        .get("timestamp_granularities")
        .and_then(|value| value.as_array())
        .is_some_and(|granularities| {
            granularities
                .iter()
                .any(|granularity| granularity.as_str() == Some(WORD_GRANULARITY))
        });
    let subtitle_requested = optional
        .get("response_format")
        .and_then(|value| value.as_str())
        .is_some_and(|format| SUBTITLE_RESPONSE_FORMATS.contains(&format));
    word_requested || subtitle_requested
}

pub fn transform_audio_request(
    model: &str,
    audio: &ProcessedAudioFile,
    optional: &Object,
) -> Result<AudioTranscriptionRequestData, StudioTranscriptionError> {
    let language_codes = optional
        .get("language")
        .and_then(|value| value.as_str())
        .filter(|language| !language.is_empty())
        .map(|language| [normalize_language_to_bcp47(language)]);
    let mode = wants_word_timestamps(optional).then_some(VerbatimMode {
        kind: TranscriptionModeType::Verbatim,
        timestamp_granularities: [TranscriptionGranularity::Word],
        diarization_mode: DiarizationMode::Speaker,
    });
    let configured = language_codes.is_some() || mode.is_some();
    let transcription_config = TranscriptionConfig {
        language_codes,
        mode,
    };
    let generation_config = configured.then_some(GenerationConfig {
        transcription_config,
    });
    let request = InteractionRequest {
        model: model.strip_prefix(MODEL_PREFIX).unwrap_or(model).to_owned(),
        input: [AudioInput {
            kind: AudioInputType::Audio,
            data: STANDARD.encode(&audio.file_content),
            mime_type: audio.content_type.clone(),
        }],
        generation_config,
    };
    let value = serde_json::to_value(request)
        .map_err(|source| StudioTranscriptionError::Json { source })?;
    let data: Object = serde_json::from_value(value)
        .map_err(|source| StudioTranscriptionError::Json { source })?;
    Ok(AudioTranscriptionRequestData {
        data: AudioTranscriptionData::Object(data),
        files: None,
        content_type: None,
    })
}
