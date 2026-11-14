use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, AudioTranscriptionRequestData,
};
use betterlite_openai::openai_base_config::Object;
use serde::Serialize;

use super::endpointing::{EndpointingConfig, endpointing_config};
use super::error::RecordsError;
use super::options::{
    DEFAULT_AUDIO_CHANNEL_COUNT, DEFAULT_ENCODING, DEFAULT_LANGUAGE_CODE, DEFAULT_MAX_ALTERNATIVES,
    DEFAULT_RESPONSE_FORMAT, DEFAULT_SAMPLE_RATE_HERTZ, bool_option, granularities_at,
    integer_option, text_option,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TranscriptFilters {
    pub verbatim_transcripts: bool,
    pub profanity_filter: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecognitionOptions {
    pub language_code: String,
    pub sample_rate_hertz: u32,
    pub encoding: String,
    pub audio_channel_count: u32,
    pub enable_automatic_punctuation: bool,
    pub enable_word_time_offsets: bool,
    pub max_alternatives: u32,
    pub model: String,
    #[serde(flatten)]
    pub transcript_filters: TranscriptFilters,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpointing_config: Option<EndpointingConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RivaRequest {
    pub recognition_config: RecognitionOptions,
    pub response_format: String,
    pub timestamp_granularities: Option<Vec<String>>,
}

impl TranscriptFilters {
    fn from_params(params: &Object) -> Result<Self, RecordsError> {
        Ok(Self {
            verbatim_transcripts: bool_option(params, "verbatim_transcripts")?.unwrap_or(false),
            profanity_filter: bool_option(params, "profanity_filter")?.unwrap_or(false),
        })
    }
}

impl RecognitionOptions {
    fn from_params(params: &Object) -> Result<Self, RecordsError> {
        Ok(Self {
            language_code: text_option(params, "language_code")?
                .unwrap_or_else(|| DEFAULT_LANGUAGE_CODE.to_owned()),
            sample_rate_hertz: integer_option(params, "sample_rate_hertz")?
                .unwrap_or(DEFAULT_SAMPLE_RATE_HERTZ),
            encoding: text_option(params, "encoding")?
                .unwrap_or_else(|| DEFAULT_ENCODING.to_owned()),
            audio_channel_count: integer_option(params, "audio_channel_count")?
                .unwrap_or(DEFAULT_AUDIO_CHANNEL_COUNT),
            enable_automatic_punctuation: bool_option(params, "enable_automatic_punctuation")?
                .unwrap_or(true),
            enable_word_time_offsets: bool_option(params, "enable_word_time_offsets")?
                .unwrap_or(false),
            max_alternatives: integer_option(params, "max_alternatives")?
                .unwrap_or(DEFAULT_MAX_ALTERNATIVES),
            model: text_option(params, "riva_model_name")?.unwrap_or_default(),
            transcript_filters: TranscriptFilters::from_params(params)?,
            endpointing_config: endpointing_config(params)?,
        })
    }
}

pub fn transform_request(optional_params: &Object) -> Result<RivaRequest, RecordsError> {
    Ok(RivaRequest {
        recognition_config: RecognitionOptions::from_params(optional_params)?,
        response_format: text_option(optional_params, "response_format")?
            .filter(|format| !format.is_empty())
            .unwrap_or_else(|| DEFAULT_RESPONSE_FORMAT.to_owned()),
        timestamp_granularities: granularities_at(
            optional_params,
            "timestamp_granularities",
            "timestamp_granularities",
        )?,
    })
}

impl RivaRequest {
    pub fn request_data(&self) -> Result<AudioTranscriptionRequestData, RecordsError> {
        let value = serde_json::to_value(self).map_err(|source| RecordsError::Value {
            field: "request",
            source,
        })?;
        let data: Object = serde_json::from_value(value).map_err(|source| RecordsError::Value {
            field: "request",
            source,
        })?;
        Ok(AudioTranscriptionRequestData {
            data: AudioTranscriptionData::Object(data),
            files: None,
            content_type: None,
        })
    }
}
