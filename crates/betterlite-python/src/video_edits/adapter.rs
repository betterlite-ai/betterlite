use std::collections::BTreeMap;
use std::fmt::Display;
use std::io::Read as _;

use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_character_calls::error::CharacterProviderFailure;
use betterlite_core::api::video_character_transport::config::{
    CharacterConfig, CharacterContext, CharacterEnvironment, LookupRequest,
};
use betterlite_core::api::video_edit_call::VideoEditConfigLookup;
use betterlite_core::api::video_edit_transport::config::{
    BaseVideoConfig, Headers, Params, RequestFile, RequestFiles, VideoEditInput, VideoEnvironment,
    VideoExtensionInput, VideoLogging, VideoReply, VideoRequestContext,
};
use betterlite_core::api::video_edit_transport::error::BoxError;
use betterlite_core::api::video_extension_call::VideoExtensionConfigLookup;
use betterlite_core::api::video_records::{CharacterObject, VideoObject};
use betterlite_core::api::video_selection::VideoConfigTag;
use betterlite_http::interactions::base::{
    ErrorClasses, Params as HttpParams, RequestHeaders, ResponseHeaders,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart as HttpPart;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_video_identifiers::multipart::{
    MediaBody, MultipartPart, video_content_type,
};
use betterlite_openai::video_edits::character::CharacterPart;
use betterlite_openai::video_edits::character::transform_video_create_character_request as create_character_request; // ast-grep-ignore: no-long-names
use betterlite_openai::video_edits::character::transform_video_get_character_request as get_character_request; // ast-grep-ignore: no-long-names
use betterlite_openai::video_edits::edit::{
    VideoEditRequest, transform_video_edit_request as edit_request,
};
use betterlite_openai::video_edits::extension::VideoExtensionRequest;
use betterlite_openai::video_edits::extension::transform_video_extension_request as extension_request; // ast-grep-ignore: no-long-names
use betterlite_openai::video_edits::response::{
    VideoReply as OpenAiReply, decode_character_reply, transform_video_response as video_response,
};
use betterlite_openai::videos::config::{VideoCredentials, complete_url, validate_environment};
use betterlite_openai::videos::error::get_error_class as provider_error_class;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::CharacterProviderError;

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

fn character_params_key(params: &HttpParams) -> Option<SecretString> {
    match params.get("api_key")? {
        JsonValue::Text(text) => Some(SecretString::from(text.clone())),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    }
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
        content_type: _,
        mut body,
    } = part;
    let filename = name.map_or_else(|| filename, ToOwned::to_owned);
    let content_type = video_content_type(&mut body, &filename);
    Ok(RequestFile {
        field,
        filename: Some(filename),
        content: body_bytes(body)?,
        content_type: Some(content_type.to_owned()),
    })
}

fn body_bytes(body: MediaBody<'_>) -> Result<Vec<u8>, CharacterProviderError> {
    match body {
        MediaBody::Bytes(bytes) => Ok(bytes.to_vec()),
        MediaBody::Text(text) => Ok(text.as_bytes().to_vec()),
        MediaBody::Stream { mut reader, .. } => {
            let mut read = Vec::new();
            reader
                .read_to_end(&mut read)
                .map_err(|source| provider_fault(&source))?;
            Ok(read)
        }
    }
}

fn character_part(
    part: CharacterPart<'_>,
    name: Option<&str>,
) -> Result<HttpPart, CharacterProviderError> {
    match part {
        CharacterPart::Text { field, text } => Ok(HttpPart {
            name: field.to_owned(),
            file_name: None,
            content_type: None,
            data: text.as_bytes().to_vec(),
        }),
        CharacterPart::File(part) => {
            let file = named_part(part, name)?;
            Ok(HttpPart {
                name: file.field,
                file_name: file.filename,
                content_type: file.content_type,
                data: file.content,
            })
        }
    }
}

impl CharacterProviderFailure for CharacterProviderError {
    fn status_code(&self) -> Option<u16> {
        self.status_code
    }

    fn message(&self) -> String {
        self.message.clone()
    }
}

impl ErrorClasses for OpenAiEditConfig {
    type Error = CharacterProviderError;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        _headers: &ResponseHeaders,
    ) -> CharacterProviderError {
        CharacterProviderError {
            status_code: Some(status_code),
            message: message.to_owned(),
        }
    }
}

impl CharacterConfig for OpenAiEditConfig {
    fn validate_environment(&self, environment: CharacterEnvironment<'_>) -> RequestHeaders {
        let credentials = self.credentials(
            environment.api_key,
            character_params_key(environment.litellm_params),
        );
        validate_environment(environment.headers, &credentials)
    }

    fn complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &HttpParams,
    ) -> Result<String, Self::Error> {
        Ok(complete_url(api_base))
    }

    fn transform_create_request(
        &self,
        name: &str,
        video: &SuppliedMedia,
        context: CharacterContext<'_>,
    ) -> Result<(String, Vec<HttpPart>), Self::Error> {
        let character = create_character_request(name, media_body(video), context.api_base);
        let mut parts = Vec::new();
        for part in character.files {
            parts.push(character_part(part, self.video_name.as_deref())?);
        }
        Ok((character.url, parts))
    }

    fn transform_get_request(
        &self,
        character_id: &str,
        context: CharacterContext<'_>,
    ) -> Result<LookupRequest, Self::Error> {
        let url = get_character_request(character_id, context.api_base)
            .map_err(|source| provider_fault(&source))?;
        Ok((url, Vec::new()))
    }

    fn transform_create_response(
        &self,
        reply: &UpstreamResponse,
        _logging: &mut LoggingState,
    ) -> Result<CharacterObject, Self::Error> {
        decode_character_reply(&reply.body).map_err(|source| provider_fault(&source))
    }

    fn transform_get_response(
        &self,
        reply: &UpstreamResponse,
        _logging: &mut LoggingState,
    ) -> Result<CharacterObject, Self::Error> {
        decode_character_reply(&reply.body).map_err(|source| provider_fault(&source))
    }
}

impl VideoEditConfigLookup for OpenAiEditConfig {
    fn video_config(&self, tag: &VideoConfigTag) -> Option<&dyn BaseVideoConfig> {
        if matches!(tag, VideoConfigTag::OpenAi) {
            Some(self)
        } else {
            None
        }
    }
}

impl VideoExtensionConfigLookup for OpenAiEditConfig {
    fn video_config(&self, tag: &VideoConfigTag) -> Option<&dyn BaseVideoConfig> {
        if matches!(tag, VideoConfigTag::OpenAi) {
            Some(self)
        } else {
            None
        }
    }
}

impl BaseVideoConfig for OpenAiEditConfig {
    fn validate_environment(&self, request: VideoEnvironment<'_>) -> Headers {
        self.edit_headers(request)
    }

    fn get_complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, BoxError> {
        Ok(complete_url(api_base))
    }

    fn get_video_edit_prefetch_params(
        &self,
        _video_id: &str,
        _context: VideoRequestContext<'_>,
    ) -> Option<(String, Params)> {
        None
    }

    fn transform_video_edit_request(
        &self,
        input: VideoEditInput<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params, Option<RequestFiles>), BoxError> {
        let request = VideoEditRequest {
            prompt: input.prompt.to_owned(),
            video_id: input.video_id.to_owned(),
            api_base: context.api_base.to_owned(),
            video_file: input.video_file.map(media_body),
            extra_body: input.extra_body.cloned(),
            prefetched_source_data: None,
        };
        let edit = edit_request(request);
        let files = match edit.files {
            Some(part) => Some(vec![named_part(part, self.video_name.as_deref())?]),
            None => None,
        };
        Ok((edit.url, edit.body, files))
    }

    fn transform_video_edit_response(
        &self,
        reply: VideoReply<'_>,
        _request_data: Option<&Params>,
    ) -> Result<VideoObject, BoxError> {
        let object = video_response(&OpenAiReply {
            body: &reply.raw_response.body,
            custom_llm_provider: reply.custom_llm_provider,
            request_data: None,
        })?;
        Ok(object)
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_extension_request(
        &self,
        input: VideoExtensionInput<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), BoxError> {
        let request = VideoExtensionRequest {
            prompt: input.prompt.to_owned(),
            video_id: input.video_id.to_owned(),
            seconds: input.seconds.to_owned(),
            api_base: context.api_base.to_owned(),
            extra_body: input.extra_body.cloned(),
        };
        let extension = extension_request(request);
        Ok((extension.url, extension.body))
    }

    // ast-grep-ignore: no-long-names
    fn transform_video_extension_response(
        &self,
        reply: VideoReply<'_>,
    ) -> Result<VideoObject, BoxError> {
        let object = video_response(&OpenAiReply {
            body: &reply.raw_response.body,
            custom_llm_provider: reply.custom_llm_provider,
            request_data: None,
        })?;
        Ok(object)
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> ProviderError {
        let pairs = headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        provider_error_class(error_message, status_code, pairs)
    }
}
