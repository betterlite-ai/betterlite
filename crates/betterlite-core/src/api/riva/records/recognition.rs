use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, AudioTranscriptionRequestData,
};
use betterlite_openai::openai_base_config::Object;
use serde_json::Value;

use super::endpointing::{EndpointingConfig, endpointing_config};
use super::error::RecordsError;
use super::options::{
    DEFAULT_AUDIO_CHANNEL_COUNT, DEFAULT_ENCODING, DEFAULT_LANGUAGE_CODE, DEFAULT_MAX_ALTERNATIVES,
    DEFAULT_RESPONSE_FORMAT, DEFAULT_SAMPLE_RATE_HERTZ, bool_option, granularity_option,
    integer_option, text_option,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranscriptFilters {
    pub verbatim_transcripts: bool,
    pub profanity_filter: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecognitionOptions {
    pub language_code: String,
    pub sample_rate_hertz: u32,
    pub encoding: String,
    pub audio_channel_count: u32,
    pub enable_automatic_punctuation: bool,
    pub enable_word_time_offsets: bool,
    pub max_alternatives: u32,
    pub model: String,
    pub transcript_filters: TranscriptFilters,
    pub endpointing_config: Option<EndpointingConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

    fn config_object(&self) -> Result<Object, RecordsError> {
        let mut config = Object::new();
        config.insert(
            "language_code".to_owned(),
            Value::from(self.language_code.as_str()),
        );
        config.insert(
            "sample_rate_hertz".to_owned(),
            Value::from(self.sample_rate_hertz),
        );
        config.insert("encoding".to_owned(), Value::from(self.encoding.as_str()));
        config.insert(
            "audio_channel_count".to_owned(),
            Value::from(self.audio_channel_count),
        );
        config.insert(
            "enable_automatic_punctuation".to_owned(),
            Value::Bool(self.enable_automatic_punctuation),
        );
        config.insert(
            "enable_word_time_offsets".to_owned(),
            Value::Bool(self.enable_word_time_offsets),
        );
        config.insert(
            "max_alternatives".to_owned(),
            Value::from(self.max_alternatives),
        );
        config.insert("model".to_owned(), Value::from(self.model.as_str()));
        config.insert(
            "verbatim_transcripts".to_owned(),
            Value::Bool(self.transcript_filters.verbatim_transcripts),
        );
        config.insert(
            "profanity_filter".to_owned(),
            Value::Bool(self.transcript_filters.profanity_filter),
        );
        if let Some(endpointing) = &self.endpointing_config {
            config.insert("endpointing_config".to_owned(), endpointing.to_value()?);
        }
        Ok(config)
    }
}

pub fn transform_request(optional_params: &Object) -> Result<RivaRequest, RecordsError> {
    Ok(RivaRequest {
        recognition_config: RecognitionOptions::from_params(optional_params)?,
        response_format: text_option(optional_params, "response_format")?
            .filter(|format| !format.is_empty())
            .unwrap_or_else(|| DEFAULT_RESPONSE_FORMAT.to_owned()),
        timestamp_granularities: granularity_option(optional_params, "timestamp_granularities")?,
    })
}

impl RivaRequest {
    pub fn request_data(&self) -> Result<AudioTranscriptionRequestData, RecordsError> {
        let mut data = Object::new();
        data.insert(
            "recognition_config".to_owned(),
            Value::Object(self.recognition_config.config_object()?),
        );
        data.insert(
            "response_format".to_owned(),
            Value::from(self.response_format.as_str()),
        );
        data.insert(
            "timestamp_granularities".to_owned(),
            self.granularities_value(),
        );
        Ok(AudioTranscriptionRequestData {
            data: AudioTranscriptionData::Object(data),
            files: None,
            content_type: None,
        })
    }

    fn granularities_value(&self) -> Value {
        let Some(names) = &self.timestamp_granularities else {
            return Value::Null;
        };
        let entries = names
            .iter()
            .map(|name| Value::from(name.as_str()))
            .collect();
        Value::Array(entries)
    }
}
