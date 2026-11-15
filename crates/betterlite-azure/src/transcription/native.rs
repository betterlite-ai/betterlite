use betterlite_http::user_url::error::InvalidUrl;
use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioFileError, AuthorizedAudioFiles, process_audio_file,
};
use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, AudioTranscriptionRequestData,
};
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use super::encoding::quote_plus;

mod reply;
mod tests;

pub use self::reply::{NativeTranscriptionReply, decode_native_transcription};

pub const COGNITIVE_SERVICES_DOMAIN: &str = "api.cognitive.microsoft.com";
pub const STT_SPEECH_DOMAIN: &str = "stt.speech.microsoft.com";
pub const STT_ENDPOINT_PATH: &str = "/speech/recognition/conversation/cognitiveservices/v1";
pub const DEFAULT_LANGUAGE: &str = "en-US";

const DEFAULT_CONTENT_TYPE: &str = "audio/wav";
const API_KEY_HEADER: &str = "Ocp-Apim-Subscription-Key";

// AzureSpeechAudioTranscriptionException: the status and headers ride on the error like BaseLLMException
#[derive(Debug, Error)]
pub enum NativeError {
    #[error("api_key is required for Azure AI Speech transcription.")]
    MissingApiKey,
    #[error(
        "api_base is required for Azure AI Speech transcription. Use a Cognitive Services endpoint like https://{{region}}.api.cognitive.microsoft.com or an STT endpoint like https://{{region}}.stt.speech.microsoft.com."
    )]
    MissingApiBase,
    #[error(
        "Azure AI Speech transcription requires a Cognitive Services or STT Speech endpoint, not an Azure OpenAI endpoint."
    )]
    AzureOpenAiEndpoint,
    #[error("Azure AI Speech transcription failed with RecognitionStatus={value}.")]
    RecognitionStatus {
        status: u16,
        headers: Headers,
        value: String,
    },
    #[error("the transcription reply is not JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the transcription reply is not a JSON object")]
    NotObject,
    #[error("the transcription field `{field}` {problem}")]
    Field {
        field: String,
        problem: &'static str,
    },
    #[error("the transcription api_base is not a URL")]
    Url {
        #[source]
        source: InvalidUrl,
    },
    #[error(transparent)]
    AudioFile(#[from] AudioFileError),
    #[error("{message}")]
    Failed {
        message: String,
        status: u16,
        headers: Headers,
    },
}

impl NativeError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::MissingApiKey => Some(401),
            Self::MissingApiBase | Self::AzureOpenAiEndpoint => Some(400),
            Self::RecognitionStatus { status, .. } | Self::Failed { status, .. } => Some(*status),
            Self::Json { .. }
            | Self::NotObject
            | Self::Field { .. }
            | Self::Url { .. }
            | Self::AudioFile(_) => None,
        }
    }

    // the raw response headers LiteLLM attaches to a failed RecognitionStatus
    #[must_use]
    pub const fn headers(&self) -> Option<&Headers> {
        match self {
            Self::RecognitionStatus { headers, .. } | Self::Failed { headers, .. } => Some(headers),
            _ => None,
        }
    }
}

// AzureSpeechAudioTranscriptionConfig.supported_openai_params answers exactly these two names
pub const SUPPORTED_OPENAI_PARAMS: &[&str] = &["language", "response_format"];

// map_openai_params copies the supported non-default params in their own order and ignores drop_params
#[must_use]
pub fn map_openai_params(
    non_default_params: &IndexMap<String, Json>,
    optional_params: IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let mut mapped = optional_params;
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            mapped.insert(name.clone(), value.clone());
        }
    }
    mapped
}

pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
    environment_key: Option<&SecretString>,
) -> Result<Headers, NativeError> {
    let key = api_key
        .or(environment_key)
        .ok_or(NativeError::MissingApiKey)?;
    let mut validated = headers.clone();
    validated.insert(API_KEY_HEADER.to_owned(), key.expose_secret().to_owned());
    validated
        .entry(String::from("Content-Type"))
        .or_insert_with(|| DEFAULT_CONTENT_TYPE.to_owned());
    validated.insert(String::from("Accept"), String::from("application/json"));
    Ok(validated)
}

pub fn complete_url(
    api_base: Option<&str>,
    environment_base: Option<&str>,
    optional_params: &IndexMap<String, Json>,
) -> Result<String, NativeError> {
    let base = api_base
        .or(environment_base)
        .ok_or(NativeError::MissingApiBase)?;
    let stt_base = resolve_stt_base_url(base)?;
    let language = text_option(optional_params, "language")?.unwrap_or(DEFAULT_LANGUAGE);
    let format = match text_option(optional_params, "response_format")? {
        Some("verbose_json") => "detailed",
        Some(_) | None => "simple",
    };
    Ok(format!(
        "{stt_base}{STT_ENDPOINT_PATH}?language={}&format={}",
        quote_plus(language),
        quote_plus(format)
    ))
}

fn text_option<'params>(
    optional_params: &'params IndexMap<String, Json>,
    name: &str,
) -> Result<Option<&'params str>, NativeError> {
    match optional_params.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.as_str())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(NativeError::Field {
                field: name.to_owned(),
                problem: "is not a string",
            })
        }
    }
}

// _resolve_stt_base_url: a Cognitive Services host becomes the regional STT host, an STT host keeps its scheme and host, any other base keeps its trimmed text
pub fn resolve_stt_base_url(api_base: &str) -> Result<String, NativeError> {
    let trimmed = api_base.trim_end_matches('/');
    let parsed = HttpxUrl::parse(trimmed).map_err(|source| NativeError::Url { source })?;
    let hostname = parsed.host.as_str();
    if hostname == COGNITIVE_SERVICES_DOMAIN
        || hostname.ends_with(&format!(".{COGNITIVE_SERVICES_DOMAIN}"))
    {
        let suffix = format!(".{COGNITIVE_SERVICES_DOMAIN}");
        let region = hostname.strip_suffix(&suffix).unwrap_or_default();
        return Ok(build_stt_base_url(region));
    }
    if hostname == STT_SPEECH_DOMAIN || hostname.ends_with(&format!(".{STT_SPEECH_DOMAIN}")) {
        return Ok(format!("{}://{hostname}", parsed.scheme));
    }
    if hostname.ends_with(".openai.azure.com") {
        return Err(NativeError::AzureOpenAiEndpoint);
    }
    Ok(trimmed.to_owned())
}

fn build_stt_base_url(region: &str) -> String {
    if region.is_empty() {
        format!("https://{STT_SPEECH_DOMAIN}")
    } else {
        format!("https://{region}.{STT_SPEECH_DOMAIN}")
    }
}

pub fn transform_request(
    audio_file: AudioFile<'_>,
    files: &dyn AuthorizedAudioFiles,
) -> Result<AudioTranscriptionRequestData, NativeError> {
    let processed = process_audio_file(audio_file, files)?;
    Ok(AudioTranscriptionRequestData {
        data: AudioTranscriptionData::Bytes(processed.file_content),
        files: None,
        content_type: Some(processed.content_type),
    })
}

// get_error_class: the caller's message, status and headers on the native exception
#[must_use]
pub fn get_error_class(message: String, status: u16, headers: Headers) -> NativeError {
    NativeError::Failed {
        message,
        status,
        headers,
    }
}
