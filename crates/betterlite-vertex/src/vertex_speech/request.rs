use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use crate::endpoint_target::region::DEFAULT_LOCATION;
use crate::model_urls::base_url::validate_vertex_location;
use crate::studio_interactions::credentials::RequestHeaders;
use crate::vertex_interactions::mint::MintAccessToken;

use super::error::VertexSpeechError;
use super::mapping::{
    DEFAULT_LANGUAGE_CODE, DEFAULT_SPEAKING_RATE, DEFAULT_VOICE_NAME, SpeechAudioConfig,
    SpeechMapping, voice_language,
};

pub const TTS_API_URL: &str = "https://texttospeech.googleapis.com/v1/text:synthesize";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SpeechInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssml: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct SpeechVoiceSettings {
    #[serde(rename = "languageCode", skip_serializing_if = "Option::is_none")]
    pub language_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    // the other fields of a Google voice (ssmlGender and the like) go through unchanged
    #[serde(flatten)]
    pub other: Object,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SpeechRequestBody {
    pub input: SpeechInput,
    pub voice: SpeechVoiceSettings,
    #[serde(rename = "audioConfig")]
    pub audio_config: SpeechAudioConfig,
}

#[derive(Debug, Clone, Copy)]
pub struct SpeechCall<'call> {
    pub input: &'call str,
    pub voice: Option<&'call str>,
    pub mapping: &'call SpeechMapping,
    pub voice_dict: Option<&'call Object>,
    pub credentials: Option<&'call ConfiguredCredentials>,
    pub project: Option<&'call str>,
    pub location: Option<&'call str>,
}

#[derive(Debug)]
pub struct SpeechRequest {
    pub body: SpeechRequestBody,
    pub headers: RequestHeaders,
}

// LiteLLM's validate_environment: the caller's headers, with JSON and UTF-8 forced over them
#[must_use]
pub fn validate_environment(caller: &IndexMap<String, String>) -> RequestHeaders {
    let mut headers: RequestHeaders = caller
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
        .collect();
    headers.insert(
        "Content-Type".to_owned(),
        SecretString::from("application/json"),
    );
    headers.insert("charset".to_owned(), SecretString::from("UTF-8"));
    headers
}

// LiteLLM's get_complete_url: a configured base wins, an empty one counts as unset
#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    api_base
        .filter(|base| !base.is_empty())
        .map_or_else(|| TTS_API_URL.to_owned(), str::to_owned)
}

pub fn validate_input(
    input: &SpeechInput,
    use_ssml: bool,
) -> Result<SpeechInput, VertexSpeechError> {
    let (text, ssml) = match (input.text.as_deref(), input.ssml.as_deref()) {
        (Some(value), _) if use_ssml => (None, Some(value)),
        (None, None) if use_ssml => return Err(VertexSpeechError::SsmlRequired),
        (Some(value), _) if value.contains("<speak>") => (None, Some(value)),
        (text, ssml) => (text, ssml),
    };
    match (text, ssml) {
        (None, None) => Err(VertexSpeechError::InputRequired),
        (Some(_), Some(_)) => Err(VertexSpeechError::BothInputs),
        _ => Ok(SpeechInput {
            text: text.map(str::to_owned),
            ssml: ssml.map(str::to_owned),
        }),
    }
}

pub async fn prepare_request(
    mint: &dyn MintAccessToken,
    call: &SpeechCall<'_>,
    caller_headers: &IndexMap<String, String>,
) -> Result<SpeechRequest, VertexSpeechError> {
    let mut headers = validate_environment(caller_headers);
    let access = mint
        .mint(call.credentials, call.project)
        .await
        .map_err(VertexSpeechError::Mint)?;
    let location = call
        .location
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_LOCATION);
    validate_vertex_location(Some(location)).map_err(VertexSpeechError::Location)?;
    headers.insert(
        "Authorization".to_owned(),
        SecretString::from(format!("Bearer {}", access.token.expose_secret())),
    );
    headers.insert(
        "x-goog-user-project".to_owned(),
        SecretString::from(access.project),
    );
    Ok(SpeechRequest {
        body: request_body(call)?,
        headers,
    })
}

fn request_body(call: &SpeechCall<'_>) -> Result<SpeechRequestBody, VertexSpeechError> {
    let input = validate_input(
        &SpeechInput {
            text: Some(call.input.to_owned()),
            ssml: None,
        },
        call.mapping.use_ssml,
    )?;
    let audio_config = call
        .mapping
        .audio_config
        .clone()
        .unwrap_or_else(|| SpeechAudioConfig {
            audio_encoding: Some(call.mapping.audio_encoding.clone()),
            speaking_rate: Some(
                call.mapping
                    .speaking_rate
                    .clone()
                    .unwrap_or_else(|| DEFAULT_SPEAKING_RATE.to_owned()),
            ),
            other: Object::new(),
        });
    Ok(SpeechRequestBody {
        input,
        voice: voice_settings(call)?,
        audio_config,
    })
}

fn voice_settings(call: &SpeechCall<'_>) -> Result<SpeechVoiceSettings, VertexSpeechError> {
    let chosen = call
        .voice_dict
        .filter(|dict| !dict.is_empty())
        .or(call.mapping.voice_dict.as_ref());
    if let Some(dict) = chosen {
        return serde_json::to_value(dict)
            .and_then(serde_json::from_value)
            .map_err(VertexSpeechError::VoiceRecord);
    }
    let Some(name) = call.voice else {
        return Ok(SpeechVoiceSettings {
            language_code: Some(DEFAULT_LANGUAGE_CODE.to_owned()),
            name: Some(DEFAULT_VOICE_NAME.to_owned()),
            other: Object::new(),
        });
    };
    Ok(SpeechVoiceSettings {
        language_code: Some(voice_language(name)),
        name: Some(name.to_owned()),
        other: Object::new(),
    })
}
