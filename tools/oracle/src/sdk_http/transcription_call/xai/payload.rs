use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, AudioTranscriptionFileValue, AudioTranscriptionFiles,
    AudioTranscriptionRequestData, TranscriptionResponse,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_provider::api::transcription_contract::{
    AudioFile, AudioTranscriptionConfig, TranscriptionContractError,
};
use betterlite_provider::api::transcription_http::config::{
    ConfigFailure, Headers, TranscriptionHttpConfig, UrlInput, Validation,
};
use serde::Deserialize;

const NO_KEY: &str = "xAI API key is required. Set XAI_API_KEY environment variable.";

// xAI's own audio transcription config, the one a native endpoint selection reaches over B2453
pub(super) struct XaiConfig;

#[derive(Deserialize)]
struct Payload {
    #[serde(default)]
    text: String,
    #[serde(default = "default_language")]
    language: String,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    words: Option<Vec<Word>>,
}

#[derive(Deserialize)]
struct Word {
    #[serde(default)]
    text: String,
    #[serde(default)]
    start: f64,
    #[serde(default)]
    end: f64,
    #[serde(default)]
    speaker: Option<i64>,
}

fn default_language() -> String {
    String::from("unknown")
}

impl AudioTranscriptionConfig for XaiConfig {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        vec![String::from("language")]
    }

    fn transform_audio_request(
        &self,
        model: &str,
        audio_file: &AudioFile<'_>,
        optional_params: &Object,
        _litellm_params: &Object,
    ) -> Result<AudioTranscriptionRequestData, TranscriptionContractError> {
        let mut form = Object::new();
        form.insert(String::from("model"), serde_json::Value::from(model));
        if let Some(serde_json::Value::Object(extra)) = optional_params.get("extra_body") {
            form.extend(extra.clone());
        }
        for (name, value) in optional_params {
            if value.is_null()
                || name == "model"
                || name == "OPENAI_TRANSCRIPTION_PARAMS"
                || name == "extra_body"
            {
                continue;
            }
            form.insert(name.clone(), value.clone());
        }
        let mut files = AudioTranscriptionFiles::new();
        files.insert(
            String::from("file"),
            AudioTranscriptionFileValue::Bytes(audio_file.content.to_vec()),
        );
        Ok(AudioTranscriptionRequestData {
            data: AudioTranscriptionData::Object(form),
            files: Some(files),
            content_type: None,
        })
    }

    fn has_native_endpoint(&self) -> bool {
        true
    }

    fn transform_audio_response(
        &self,
        raw_response: &UpstreamResponse,
    ) -> Result<TranscriptionResponse, TranscriptionContractError> {
        let payload: Payload = serde_json::from_slice(&raw_response.body)
            .map_err(|_error| TranscriptionContractError::UnexpectedResponseTransform)?;
        let mut extra = Object::new();
        extra.insert(String::from("task"), serde_json::Value::from("transcribe"));
        extra.insert(
            String::from("language"),
            serde_json::Value::from(payload.language.clone()),
        );
        if let Some(duration) = payload.duration {
            extra.insert(String::from("duration"), serde_json::Value::from(duration));
        }
        insert_words(&mut extra, payload.words.as_deref());
        extra.insert(
            String::from("_hidden_params"),
            serde_json::Value::Object(hidden_params(&payload)),
        );
        Ok(TranscriptionResponse {
            text: Some(payload.text),
            extra,
        })
    }
}

fn insert_words(extra: &mut Object, words: Option<&[Word]>) {
    let Some(words) = words else {
        return;
    };
    let mut mapped = Vec::new();
    for word in words {
        let mut entry = Object::new();
        entry.insert(
            String::from("word"),
            serde_json::Value::from(word.text.clone()),
        );
        entry.insert(String::from("start"), serde_json::Value::from(word.start));
        entry.insert(String::from("end"), serde_json::Value::from(word.end));
        if let Some(speaker) = word.speaker {
            entry.insert(String::from("speaker"), serde_json::Value::from(speaker));
        }
        mapped.push(serde_json::Value::Object(entry));
    }
    extra.insert(String::from("words"), serde_json::Value::Array(mapped));
}

// model_dump(mode="json") of the payload, plus the duration cost tracking reads
fn hidden_params(payload: &Payload) -> Object {
    let mut hidden = Object::new();
    hidden.insert(
        String::from("text"),
        serde_json::Value::from(payload.text.clone()),
    );
    hidden.insert(
        String::from("language"),
        serde_json::Value::from(payload.language.clone()),
    );
    hidden.insert(
        String::from("duration"),
        payload
            .duration
            .map_or(serde_json::Value::Null, serde_json::Value::from),
    );
    if let Some(duration) = payload.duration {
        hidden.insert(
            String::from("audio_transcription_duration"),
            serde_json::Value::from(duration),
        );
    }
    hidden
}

impl TranscriptionHttpConfig for XaiConfig {
    fn validate_environment(&self, input: &Validation<'_>) -> Result<Headers, ConfigFailure> {
        let Some(key) = input.api_key else {
            return Err(ConfigFailure {
                message: String::from(NO_KEY),
            });
        };
        let mut headers = input.headers.clone();
        headers.insert(String::from("Authorization"), format!("Bearer {key}"));
        Ok(headers)
    }

    fn complete_transcription_url(&self, input: &UrlInput<'_>) -> Result<String, ConfigFailure> {
        let base = input
            .api_base
            .unwrap_or("https://api.x.ai")
            .trim_end_matches('/');
        let normalized = base.strip_suffix("/v1").unwrap_or(base);
        Ok(format!("{normalized}/v1/stt"))
    }
}
