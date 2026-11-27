use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_character_calls::error::CharacterProviderFailure;
use betterlite_core::api::video_character_transport::config::{
    CharacterConfig, CharacterContext, CharacterEnvironment, LookupRequest,
};
use betterlite_core::api::video_records::CharacterObject;
use betterlite_http::interactions::base::{
    ErrorClasses, Params as HttpParams, RequestHeaders, ResponseHeaders,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart as HttpPart;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::video_edits::character::CharacterPart;
use betterlite_openai::video_edits::character::transform_video_create_character_request as create_character_request; // ast-grep-ignore: no-long-names
use betterlite_openai::video_edits::character::transform_video_get_character_request as get_character_request; // ast-grep-ignore: no-long-names
use betterlite_openai::video_edits::response::decode_character_reply;
use betterlite_openai::videos::config::{complete_url, validate_environment};
use secrecy::SecretString;

use crate::video_edits::error::CharacterProviderError;

use super::{OpenAiEditConfig, media_body, named_part, provider_fault, record};

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
        CharacterPart::File(file_part) => {
            let file = named_part(file_part, name)?;
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
        let character =
            decode_character_reply(&reply.body).map_err(|source| provider_fault(&source))?;
        record(&character)
    }

    // LiteLLM decodes a get character reply exactly like a create reply
    fn transform_get_response(
        &self,
        reply: &UpstreamResponse,
        logging: &mut LoggingState,
    ) -> Result<CharacterObject, Self::Error> {
        self.transform_create_response(reply, logging)
    }
}
