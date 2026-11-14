use betterlite_openai::api::audio_file_input::{
    AudioFile, AuthorizedAudioFiles, process_audio_file,
};
use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, AudioTranscriptionRequestData,
};
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use url::form_urlencoded;

use super::error::DeepgramTranscriptionError;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["language"];

// LiteLLM keeps these two out of the query: the model has its own pair and the other is internal
const RESERVED_PARAMS: [&str; 2] = ["model", "OPENAI_TRANSCRIPTION_PARAMS"];

#[derive(Clone, Debug)]
pub struct DeepgramTranscriptionConfig {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

#[must_use]
pub fn map_openai_params(
    non_default_params: &IndexMap<String, Value>,
    mut optional_params: IndexMap<String, Value>,
) -> IndexMap<String, Value> {
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
    optional_params
}

// the audio goes out as the raw request body, with no multipart files
pub fn transform_audio_transcription_request(
    audio_file: AudioFile<'_>,
    files: &dyn AuthorizedAudioFiles,
) -> Result<AudioTranscriptionRequestData, DeepgramTranscriptionError> {
    let processed = process_audio_file(audio_file, files)?;
    Ok(AudioTranscriptionRequestData {
        data: AudioTranscriptionData::Bytes(processed.file_content),
        files: None,
        content_type: None,
    })
}

impl DeepgramTranscriptionConfig {
    // an explicit base, even an empty one, replaces the configured base
    #[must_use]
    pub fn complete_url(
        &self,
        api_base: Option<&str>,
        model: &str,
        optional_params: &IndexMap<String, Value>,
    ) -> String {
        let base = api_base.unwrap_or(&self.api_base).trim_end_matches('/');
        let mut query = form_urlencoded::Serializer::new(String::new());
        query.append_pair("model", model);
        for (name, value) in optional_params {
            let reserved = RESERVED_PARAMS.contains(&name.as_str())
                || SUPPORTED_OPENAI_PARAMS.contains(&name.as_str());
            if value.is_null() || reserved {
                continue;
            }
            match value {
                Value::String(text) => query.append_pair(name, text),
                Value::Bool(flag) => query.append_pair(name, if *flag { "true" } else { "false" }),
                other => query.append_pair(name, &other.to_string()),
            };
        }
        // Python's quote_plus leaves ~ alone and encodes *, the reverse of form_urlencoded
        let query = query.finish().replace('*', "%2A").replace("%7E", "~");
        format!("{base}/listen?{query}")
    }

    // the caller's headers are dropped, and a missing key prints as the text None like LiteLLM's f-string
    #[must_use]
    pub fn validate_environment(&self, api_key: Option<&SecretString>) -> Headers {
        let key = api_key
            .or(self.api_key.as_ref())
            .map_or("None", ExposeSecret::expose_secret);
        Headers::from([("Authorization".to_owned(), format!("Token {key}"))])
    }
}
