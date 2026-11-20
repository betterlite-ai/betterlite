use betterlite_http::interactions::base::{ErrorClasses, Params, RequestHeaders};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_logging::logging_state::LoggingState;
use secrecy::SecretString;

use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::video_records::CharacterObject;

// the four arguments LiteLLM hands validate_environment, with the caller's headers already in the map it may rewrite
pub struct CharacterEnvironment<'call> {
    pub api_key: Option<&'call SecretString>,
    pub headers: RequestHeaders,
    pub model: &'call str,
    pub litellm_params: &'call Params,
}

// the last three arguments of every character request transform
pub struct CharacterContext<'call> {
    pub api_base: &'call str,
    pub litellm_params: &'call Params,
    pub headers: &'call RequestHeaders,
}

// the BaseVideoConfig members the four character handlers call; a provider adapter implements them
pub trait CharacterConfig: ErrorClasses {
    fn validate_environment(&self, environment: CharacterEnvironment<'_>) -> RequestHeaders;

    fn complete_url(
        &self,
        model: &str,
        api_base: Option<&str>,
        litellm_params: &Params,
    ) -> Result<String, Self::Error>;

    fn transform_create_request(
        &self,
        name: &str,
        video: &SuppliedMedia,
        context: CharacterContext<'_>,
    ) -> Result<(String, Vec<MultipartPart>), Self::Error>;

    fn transform_get_request(
        &self,
        character_id: &str,
        context: CharacterContext<'_>,
    ) -> Result<(String, Vec<(String, String)>), Self::Error>;

    fn transform_create_response(
        &self,
        reply: &UpstreamResponse,
        logging: &mut LoggingState,
    ) -> Result<CharacterObject, Self::Error>;

    fn transform_get_response(
        &self,
        reply: &UpstreamResponse,
        logging: &mut LoggingState,
    ) -> Result<CharacterObject, Self::Error>;
}
