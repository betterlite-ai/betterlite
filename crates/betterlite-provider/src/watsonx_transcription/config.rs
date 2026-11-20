use std::sync::{Mutex, PoisonError};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, AudioTranscriptionFileValue, AudioTranscriptionFiles,
    AudioTranscriptionRequestData, TranscriptionResponse,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::watsonx_audio::WatsonxAudioConfig;
use betterlite_openai::watsonx_audio::error::WatsonxAudioError;
// ast-grep-ignore: no-long-names
use betterlite_openai::watsonx_audio::reply::decode_watsonx_transcription_reply;
use betterlite_openai::watsonx_audio::request::{
    SUPPORTED_OPENAI_PARAMS, WatsonxTranscriptionRequest,
};
use betterlite_openai::watsonx_auth::WatsonxAuth;
use betterlite_openai::watsonx_auth::transport::WatsonxTransport;
use http::header::HeaderMap;

use super::error::WatsonxTranscriptionError;
use crate::api::transcription_contract::{
    AudioFile, AudioTranscriptionConfig, TranscriptionContractError,
};
use crate::api::transcription_http::config::{
    ConfigFailure, Headers, TranscriptionHttpConfig, UrlInput, Validation,
};

// B5842 supplies the body and the reply; this adapter hands each of its failures back to the operation
pub(super) struct BoundConfig<'auth, T> {
    auth: &'auth WatsonxAuth<T>,
    headers: Headers,
    failure: Mutex<Option<WatsonxAudioError>>,
}

impl<'auth, T> BoundConfig<'auth, T> {
    pub(super) const fn new(auth: &'auth WatsonxAuth<T>, headers: Headers) -> Self {
        Self {
            auth,
            headers,
            failure: Mutex::new(None),
        }
    }

    #[must_use]
    pub(super) fn take_failure(&self) -> Option<WatsonxAudioError> {
        self.failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

// auth resolves asynchronously before the executor runs, so its result arrives already validated
pub(super) fn header_index(headers: &HeaderMap) -> Result<Headers, WatsonxTranscriptionError> {
    let mut resolved = Headers::new();
    for (name, value) in headers {
        let text = value
            .to_str()
            .map_err(|source| WatsonxTranscriptionError::Header {
                name: name.as_str().to_owned(),
                source,
            })?;
        resolved.insert(name.as_str().to_owned(), text.to_owned());
    }
    Ok(resolved)
}

impl<T: WatsonxTransport> AudioTranscriptionConfig for BoundConfig<'_, T> {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        SUPPORTED_OPENAI_PARAMS.map(str::to_owned).to_vec()
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
        let transformed = WatsonxAudioConfig { auth: self.auth }.transform_audio_request(
            model,
            processed,
            optional_params,
        );
        match transformed {
            Ok(request) => Ok(request_data(request)),
            Err(error) => {
                *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
                Err(TranscriptionContractError::MissingRequestTransform)
            }
        }
    }

    fn transform_audio_response(
        &self,
        raw_response: &UpstreamResponse,
    ) -> Result<TranscriptionResponse, TranscriptionContractError> {
        // ast-grep-ignore: no-long-names
        match decode_watsonx_transcription_reply(&raw_response.body) {
            Ok(response) => Ok(response),
            Err(error) => {
                *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
                Err(TranscriptionContractError::UnexpectedResponseTransform)
            }
        }
    }
}

impl<T: WatsonxTransport> TranscriptionHttpConfig for BoundConfig<'_, T> {
    fn validate_environment(&self, _input: &Validation<'_>) -> Result<Headers, ConfigFailure> {
        Ok(self.headers.clone())
    }

    fn complete_transcription_url(&self, input: &UrlInput<'_>) -> Result<String, ConfigFailure> {
        let complete = WatsonxAudioConfig { auth: self.auth }
            .get_complete_url(input.api_base, input.optional_params);
        match complete {
            Ok(url) => Ok(url),
            Err(error) => {
                *self.failure.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
                Err(ConfigFailure {
                    message: String::from("the watsonx audio endpoint URL could not be built"),
                })
            }
        }
    }
}

// B2453's files map carries bytes only, so the prepared filename and media type never reach the part
fn request_data(request: WatsonxTranscriptionRequest) -> AudioTranscriptionRequestData {
    let mut fields = Object::new();
    for (name, value) in request.fields {
        fields.insert(name, value);
    }
    AudioTranscriptionRequestData {
        data: AudioTranscriptionData::Object(fields),
        files: Some(AudioTranscriptionFiles::from([(
            String::from("file"),
            AudioTranscriptionFileValue::Bytes(request.file.file_content),
        )])),
        content_type: None,
    }
}
