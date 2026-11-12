use std::error::Error;

use betterlite_openai::api::image_contracts::records::{
    ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse,
};
use betterlite_openai::error::ProviderError;
use secrecy::SecretString;

use crate::api::image_contracts::error::{
    ImageContractError, NOT_IMPLEMENTED_REQUEST, NOT_IMPLEMENTED_RESPONSE,
};
use crate::api::image_contracts::shared::{ImageEnvironment, ImageUrlRequest, base_exception};

#[derive(Debug, Clone, Copy)]
pub struct ImageGenerationRequest<'request> {
    pub model: &'request str,
    pub prompt: &'request str,
    pub optional_params: &'request ImageParams,
    pub litellm_params: &'request ImageParams,
    pub headers: &'request ImageHeaders,
}

#[derive(Debug)]
pub struct ImageGenerationReply<'request> {
    pub model: &'request str,
    pub raw_response: &'request ImageHttpResponse,
    pub model_response: &'request mut ImageResponse,
    pub request_data: &'request ImageParams,
    pub optional_params: &'request ImageParams,
    pub litellm_params: &'request ImageParams,
    pub api_key: Option<&'request SecretString>,
    pub json_mode: Option<bool>,
}

// LiteLLM's BaseImageGenerationConfig: two required transforms and six inherited defaults
pub trait BaseImageGenerationConfig: Send + Sync {
    type Error: Error + Send + Sync + 'static + From<ImageContractError>;

    fn get_supported_openai_params(&self, model: &str) -> Vec<String>;

    fn map_openai_params(
        &self,
        non_default_params: &ImageParams,
        optional_params: ImageParams,
        model: &str,
        drop_params: bool,
    ) -> ImageParams;

    fn get_complete_url(&self, request: ImageUrlRequest<'_>) -> String {
        request.api_base.unwrap_or_default().to_owned()
    }

    fn validate_environment(&self, _request: ImageEnvironment<'_>) -> ImageHeaders {
        ImageHeaders::new()
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &ImageHeaders,
    ) -> ProviderError {
        base_exception(error_message, status_code, headers)
    }

    // ast-grep-ignore: no-long-names
    fn transform_image_generation_request(
        &self,
        _request: ImageGenerationRequest<'_>,
    ) -> Result<ImageParams, Self::Error> {
        Err(Self::Error::from(ImageContractError::NotImplemented {
            message: NOT_IMPLEMENTED_REQUEST,
        }))
    }

    // ast-grep-ignore: no-long-names
    fn transform_image_generation_response(
        &self,
        _reply: ImageGenerationReply<'_>,
    ) -> Result<ImageResponse, Self::Error> {
        Err(Self::Error::from(ImageContractError::NotImplemented {
            message: NOT_IMPLEMENTED_RESPONSE,
        }))
    }

    fn use_multipart_form_data(&self) -> bool {
        false
    }
}
