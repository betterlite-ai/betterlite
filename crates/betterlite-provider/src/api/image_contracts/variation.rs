use std::error::Error;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageFile, ImageHeaders, ImageHttpResponse, ImageParams, ImageRequestFields, ImageResponse,
};
use secrecy::SecretString;

use crate::api::image_contracts::error::{
    ImageContractError, NOT_IMPLEMENTED_REQUEST, NOT_IMPLEMENTED_RESPONSE,
};
use crate::api::image_contracts::shared::{ImageEnvironment, ImageUrlRequest};

#[derive(Debug, Clone, Copy)]
pub struct ImageVariationRequest<'request> {
    pub model: Option<&'request str>,
    pub image: &'request ImageFile,
    pub optional_params: &'request ImageParams,
    pub headers: &'request ImageHeaders,
}

#[derive(Debug)]
pub struct ImageVariationReply<'request> {
    pub model: Option<&'request str>,
    pub raw_response: &'request ImageHttpResponse,
    pub model_response: &'request mut ImageResponse,
    pub request_data: &'request ImageParams,
    pub image: &'request ImageFile,
    pub optional_params: &'request ImageParams,
    pub litellm_params: &'request ImageParams,
    pub api_key: Option<&'request SecretString>,
}

#[derive(Debug, Clone, Copy)]
pub struct ImageVariationChat<'request> {
    pub model: &'request str,
    pub messages: &'request [Json],
    pub optional_params: &'request ImageParams,
    pub litellm_params: &'request ImageParams,
    pub headers: &'request ImageHeaders,
}

// LiteLLM's BaseImageVariationConfig: image variation transforms plus the chat hooks it turns into errors
#[async_trait]
pub trait BaseImageVariationConfig: Send + Sync {
    type Error: Error + Send + Sync + 'static + From<ImageContractError>;

    fn get_supported_openai_params(&self, model: &str) -> Vec<String>;

    // ast-grep-ignore: no-long-names
    fn transform_request_image_variation(
        &self,
        request: ImageVariationRequest<'_>,
    ) -> Result<ImageRequestFields, Self::Error>;

    // ast-grep-ignore: no-long-names
    async fn async_transform_response_image_variation(
        &self,
        reply: ImageVariationReply<'_>,
    ) -> Result<ImageResponse, Self::Error>;

    // ast-grep-ignore: no-long-names
    fn transform_response_image_variation(
        &self,
        reply: ImageVariationReply<'_>,
    ) -> Result<ImageResponse, Self::Error>;

    fn get_complete_url(&self, request: ImageUrlRequest<'_>) -> String {
        request.api_base.unwrap_or_default().to_owned()
    }

    #[cfg_attr(test, mutants::skip)]
    fn validate_environment(&self, _request: ImageEnvironment<'_>) -> ImageHeaders {
        ImageHeaders::new()
    }

    fn transform_request(
        &self,
        _request: ImageVariationChat<'_>,
    ) -> Result<ImageParams, Self::Error> {
        Err(Self::Error::from(ImageContractError::NotImplemented {
            message: NOT_IMPLEMENTED_REQUEST,
        }))
    }

    fn transform_response(
        &self,
        _reply: ImageVariationReply<'_>,
    ) -> Result<ImageResponse, Self::Error> {
        Err(Self::Error::from(ImageContractError::NotImplemented {
            message: NOT_IMPLEMENTED_RESPONSE,
        }))
    }
}
