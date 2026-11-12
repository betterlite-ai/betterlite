use std::error::Error;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageClassEntry, ImageClassKind, ImageConfig, ImageConfigValue, ImageFile, ImageFiles,
    ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse,
};
use betterlite_openai::error::ProviderError;
use secrecy::SecretString;

use crate::api::image_contracts::error::ImageContractError;
use crate::api::image_contracts::shared::base_exception;

#[derive(Debug)]
pub struct ImageEditRequest<'request> {
    pub model: &'request str,
    pub prompt: Option<&'request str>,
    pub image: Option<&'request ImageFile>,
    pub optional_params: ImageParams,
    pub litellm_params: &'request ImageParams,
    pub headers: ImageHeaders,
}

#[derive(Debug, Clone, Copy)]
pub struct ImageEditEnvironment<'request> {
    pub headers: &'request ImageHeaders,
    pub model: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub litellm_params: Option<&'request ImageParams>,
    pub api_base: Option<&'request str>,
}

#[derive(Debug)]
pub struct ImageEditReply<'request> {
    pub model: &'request str,
    pub raw_response: &'request ImageHttpResponse,
}

// LiteLLM's BaseImageEditConfig: the required transforms and inherited URL, bridge and config defaults
#[async_trait]
pub trait BaseImageEditConfig: Send + Sync {
    type Error: Error + Send + Sync + 'static + From<ImageContractError>;

    fn get_supported_openai_params(&self, model: &str) -> Vec<String>;

    fn map_openai_params(
        &self,
        image_edit_optional_params: &ImageParams,
        model: &str,
        drop_params: bool,
    ) -> ImageParams;

    fn transform_image_edit_request(
        &self,
        request: ImageEditRequest<'_>,
    ) -> Result<(ImageParams, ImageFiles), Self::Error>;

    fn transform_image_edit_response(
        &self,
        reply: ImageEditReply<'_>,
    ) -> Result<ImageResponse, Self::Error>;

    #[cfg_attr(test, mutants::skip)]
    fn validate_environment(&self, _request: ImageEditEnvironment<'_>) -> ImageHeaders {
        ImageHeaders::new()
    }

    fn get_complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &ImageParams,
    ) -> Result<String, Self::Error> {
        let Some(base) = api_base else {
            return Err(Self::Error::from(ImageContractError::ApiBaseRequired));
        };
        Ok(base.to_owned())
    }

    // ast-grep-ignore: no-long-names
    async fn async_transform_image_edit_request(
        &self,
        request: ImageEditRequest<'_>,
    ) -> Result<(ImageParams, ImageFiles), Self::Error> {
        let copied = ImageEditRequest {
            model: request.model,
            prompt: request.prompt,
            image: request.image,
            optional_params: request.optional_params.clone(),
            litellm_params: request.litellm_params,
            headers: request.headers.clone(),
        };
        self.transform_image_edit_request(copied)
    }

    // ast-grep-ignore: no-long-names
    fn finalize_image_edit_request_data(
        &self,
        data: ImageParams,
        _resolved_request_url: &str,
    ) -> ImageParams {
        data
    }

    fn use_multipart_form_data(&self) -> bool {
        true
    }

    #[must_use]
    fn get_config(entries: &[ImageClassEntry]) -> ImageConfig {
        entries
            .iter()
            .filter(|entry| !entry.inherited)
            .filter(|entry| !entry.name.starts_with("__") && !entry.name.starts_with("_abc"))
            .filter_map(|entry| match entry.kind {
                ImageClassKind::Value if !matches!(entry.value, Json::Null) => Some((
                    entry.name.clone(),
                    ImageConfigValue::Value {
                        value: entry.value.clone(),
                    },
                )),
                ImageClassKind::Property => Some((entry.name.clone(), ImageConfigValue::Property)),
                ImageClassKind::CallableObject => {
                    Some((entry.name.clone(), ImageConfigValue::CallableObject))
                }
                ImageClassKind::Value
                | ImageClassKind::Function
                | ImageClassKind::BuiltinFunction
                | ImageClassKind::ClassMethod
                | ImageClassKind::StaticMethod => None,
            })
            .collect()
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &ImageHeaders,
    ) -> ProviderError {
        base_exception(error_message, status_code, headers)
    }
}
