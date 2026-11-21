use betterlite_config::sections::xai_identity::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::api::batch_records::WireDecodeLimits;
use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, AudioTranscriptionFileValue, AudioTranscriptionFiles,
    AudioTranscriptionRequestData, TranscriptionResponse,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::xai_stt::{
    FormText, FormValue, SUPPORTED_OPENAI_PARAMS, TranscriptionResponse as XaiSttResponse,
    authorization_headers, decode_xai_stt_reply, stt_url, transcription_form,
};
use betterlite_provider::api::transcription_contract::{
    AudioFile, AudioTranscriptionConfig, TranscriptionContractError,
};
use betterlite_provider::api::transcription_http::config::{
    ConfigFailure, Headers, TranscriptionHttpConfig, UrlInput, Validation,
};
use http::header::{AUTHORIZATION, HeaderMap};
use indexmap::IndexMap;
use secrecy::SecretString;

// the reply limits the xai_stt operation decodes with
const LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 1 << 20,
    max_event_bytes: 1 << 10,
    max_items: 1000,
};

// the names LiteLLM never sends as form fields, whichever map carries them
const EXCLUDED_PARAMS: [&str; 3] = ["model", "OPENAI_TRANSCRIPTION_PARAMS", "extra_body"];

// xAI's own audio transcription config, the one a native endpoint selection reaches over B2453
pub(super) struct XaiConfig;

impl AudioTranscriptionConfig for XaiConfig {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        SUPPORTED_OPENAI_PARAMS
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    fn transform_audio_request(
        &self,
        model: &str,
        audio_file: &AudioFile<'_>,
        optional_params: &Object,
        _litellm_params: &Object,
    ) -> Result<AudioTranscriptionRequestData, TranscriptionContractError> {
        let processed = ProcessedAudioFile {
            file_content: audio_file.content.to_vec(),
            filename: audio_file.filename.to_owned(),
            content_type: audio_file.content_type.to_owned(),
        };
        let maps = form_params(optional_params)?;
        let form = transcription_form(model, &processed, &maps.extra_body, &maps.direct);
        let mut data = Object::new();
        for (name, value) in form.fields {
            let converted = match value.form_text() {
                FormText::Scalar(single) => serde_json::Value::String(single),
                FormText::Repeated(many) => serde_json::Value::Array(
                    many.into_iter().map(serde_json::Value::String).collect(),
                ),
            };
            data.insert(name, converted);
        }
        let mut files = AudioTranscriptionFiles::new();
        files.insert(
            String::from("file"),
            AudioTranscriptionFileValue::Bytes(form.file.data),
        );
        Ok(AudioTranscriptionRequestData {
            data: AudioTranscriptionData::Object(data),
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
        let decoded = decode_xai_stt_reply(raw_response, &LIMITS)
            .map_err(|_error| TranscriptionContractError::UnexpectedResponseTransform)?;
        contract_response(decoded)
    }
}

// the xai_stt record as the openai contract B1082 reads, with LiteLLM's hidden params
fn contract_response(
    decoded: XaiSttResponse,
) -> Result<TranscriptionResponse, TranscriptionContractError> {
    let mut extra = Object::new();
    extra.insert(
        String::from("task"),
        serde_json::to_value(decoded.task)
            .map_err(|_error| TranscriptionContractError::UnexpectedResponseTransform)?,
    );
    extra.insert(
        String::from("language"),
        serde_json::Value::from(decoded.language),
    );
    if let Some(duration) = decoded.duration {
        extra.insert(String::from("duration"), serde_json::Value::from(duration));
    }
    if let Some(words) = decoded.words {
        extra.insert(
            String::from("words"),
            serde_json::to_value(words)
                .map_err(|_error| TranscriptionContractError::UnexpectedResponseTransform)?,
        );
    }
    extra.insert(
        String::from("_hidden_params"),
        serde_json::to_value(decoded.metadata)
            .map_err(|_error| TranscriptionContractError::UnexpectedResponseTransform)?,
    );
    Ok(TranscriptionResponse {
        text: Some(decoded.text),
        extra,
    })
}

// extra_body flattens first and a direct option of the same name replaces it in place
struct FormMaps {
    extra_body: IndexMap<String, FormValue>,
    direct: IndexMap<String, FormValue>,
}

fn form_params(params: &Object) -> Result<FormMaps, TranscriptionContractError> {
    let extra_body = match params.get("extra_body") {
        Some(serde_json::Value::Object(fields)) => form_map(fields)?,
        Some(_) | None => IndexMap::new(),
    };
    Ok(FormMaps {
        extra_body,
        direct: form_map(params)?,
    })
}

fn form_map(fields: &Object) -> Result<IndexMap<String, FormValue>, TranscriptionContractError> {
    let mut mapped = IndexMap::new();
    for (name, value) in fields {
        if EXCLUDED_PARAMS.contains(&name.as_str()) {
            continue;
        }
        let parsed = serde_json::from_value::<FormValue>(value.clone())
            .map_err(|_error| TranscriptionContractError::UnexpectedResponseTransform)?;
        mapped.insert(name.clone(), parsed);
    }
    Ok(mapped)
}

impl TranscriptionHttpConfig for XaiConfig {
    fn validate_environment(&self, input: &Validation<'_>) -> Result<Headers, ConfigFailure> {
        let key = input.api_key.map(SecretString::from);
        let authorization =
            authorization_headers(HeaderMap::new(), key.as_ref(), &Settings::default()).map_err(
                |error| ConfigFailure {
                    message: error.to_string(),
                },
            )?;
        let value = authorization
            .get(AUTHORIZATION)
            .ok_or_else(|| ConfigFailure {
                message: String::from("the xAI authorization header is missing"),
            })?
            .to_str()
            .map_err(|error| ConfigFailure {
                message: error.to_string(),
            })?;
        let mut headers = input.headers.clone();
        headers.insert(String::from("Authorization"), value.to_owned());
        Ok(headers)
    }

    fn complete_transcription_url(&self, input: &UrlInput<'_>) -> Result<String, ConfigFailure> {
        Ok(stt_url(input.api_base, &Settings::default()))
    }
}
