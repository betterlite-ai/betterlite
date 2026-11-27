use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_edit_transport::config::{Headers, RequestFile, VideoEnvironment};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::multipart::{
    MediaBody, MultipartPart, video_content_type,
};
use betterlite_openai::videos::config::{VideoCredentials, validate_environment};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::error::CharacterProviderError;

mod character;
mod edit;

// LiteLLM's OpenAIVideoConfig over B525's free functions: the same identity every character, edit and extension call reads
pub(crate) struct OpenAiEditConfig {
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) global_api_key: Option<SecretString>,
    pub(crate) openai_key: Option<SecretString>,
    // the caller's file name, which the payload knew and the core's media value does not carry
    pub(crate) video_name: Option<String>,
}

impl OpenAiEditConfig {
    fn credentials(
        &self,
        api_key: Option<&SecretString>,
        params_api_key: Option<SecretString>,
    ) -> VideoCredentials {
        VideoCredentials {
            api_key: api_key.cloned(),
            params_api_key,
            global_api_key: self.global_api_key.clone(),
            openai_key: self.openai_key.clone(),
            environment_api_key: environment_key(&self.environment),
        }
    }

    // LiteLLM updates the validated headers with extra_headers a second time, so the caller's values win
    fn edit_headers(&self, request: VideoEnvironment<'_>) -> Headers {
        let credentials = self.credentials(
            request.api_key,
            request.litellm_params.and_then(edit_params_key),
        );
        let input = request
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
            .collect();
        validate_environment(input, &credentials)
            .into_iter()
            .map(|(name, value)| (name, value.expose_secret().to_owned()))
            .collect()
    }
}

fn environment_key(environment: &BTreeMap<String, String>) -> Option<SecretString> {
    environment
        .get("OPENAI_API_KEY")
        .filter(|value| !value.is_empty())
        .map(|value| SecretString::from(value.clone()))
}

fn edit_params_key(params: &IndexMap<String, Json>) -> Option<SecretString> {
    match params.get("api_key")? {
        Json::Text(text) => Some(SecretString::from(text.clone())),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// LiteLLM's own record type crosses from the provider crate into the core's as the same JSON fields
fn record<T, V>(value: &V) -> Result<T, CharacterProviderError>
where
    T: DeserializeOwned,
    V: Serialize,
{
    let encoded = serde_json::to_value(value).map_err(|source| provider_fault(&source))?;
    let decoded = serde_json::from_value(encoded).map_err(|source| provider_fault(&source))?;
    Ok(decoded)
}

fn provider_fault<E>(error: &E) -> CharacterProviderError
where
    E: Display,
{
    CharacterProviderError {
        status_code: None,
        message: error.to_string(),
    }
}

fn media_body(media: &SuppliedMedia) -> MediaBody<'_> {
    match media {
        SuppliedMedia::Bytes(bytes) => MediaBody::Bytes(bytes),
        SuppliedMedia::Text(text) => MediaBody::Text(text),
        SuppliedMedia::File(file) => MediaBody::Bytes(&file.content),
        SuppliedMedia::List(items) => items.first().map_or(MediaBody::Text(""), media_body),
    }
}

// the multipart entry B525 built, under the caller's own file name when the payload knew one
fn named_part(
    part: MultipartPart<'_>,
    name: Option<&str>,
) -> Result<RequestFile, CharacterProviderError> {
    let MultipartPart {
        field,
        filename,
        mut body,
        ..
    } = part;
    let chosen_filename = name.map_or_else(|| filename, ToOwned::to_owned);
    let content_type = video_content_type(&mut body, &chosen_filename);
    Ok(RequestFile {
        field,
        filename: Some(chosen_filename),
        content: body_bytes(body)?,
        content_type: Some(content_type.to_owned()),
    })
}

fn body_bytes(body: MediaBody<'_>) -> Result<Vec<u8>, CharacterProviderError> {
    match body {
        MediaBody::Bytes(bytes) => Ok(bytes.to_vec()),
        MediaBody::Text(text) => Ok(text.as_bytes().to_vec()),
        MediaBody::Stream { reader, .. } => {
            let mut read = Vec::new();
            reader
                .read_to_end(&mut read)
                .map_err(|source| provider_fault(&source))?;
            Ok(read)
        }
    }
}
