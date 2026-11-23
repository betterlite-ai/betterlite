use async_trait::async_trait;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageFiles, ImageHeaders, ImageParams, ImageResponse,
};
use betterlite_openai::credentials::OpenAiCredentials;
use betterlite_openai::image_edits::environment;
use betterlite_openai::image_edits::error::ImageEditError as OpenAiEditError;
use betterlite_openai::image_edits::family::{
    ImageEditFamily, SUPPORTED_OPENAI_PARAMS, map_openai_params,
};
use betterlite_openai::image_edits::file::ImageEditFile;
use betterlite_openai::image_edits::request::{
    ImageEditRequest as OpenAiEditRequest, transform_image_edit_request,
};
use betterlite_openai::image_edits::response::transform_image_edit_response;
use betterlite_provider::api::image_contracts::edit::{
    BaseImageEditConfig, ImageEditEnvironment, ImageEditReply, ImageEditRequest,
};
use betterlite_provider::api::image_contracts::error::ImageContractError;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

#[derive(Debug, Error)]
pub(super) enum OpenAiConfigError {
    #[error(transparent)]
    Contract(#[from] ImageContractError),
    #[error(transparent)]
    Edit(#[from] OpenAiEditError),
}

// B2451's call carries one image, so the config holds every file image_edit received, in order
pub(super) struct OpenAiConfig {
    pub(super) family: ImageEditFamily,
    pub(super) images: Vec<ImageEditFile>,
    pub(super) mask: Option<ImageEditFile>,
}

#[async_trait]
impl BaseImageEditConfig for OpenAiConfig {
    type Error = OpenAiConfigError;

    fn get_supported_openai_params(&self, _model: &str) -> Vec<String> {
        SUPPORTED_OPENAI_PARAMS.map(str::to_owned).to_vec()
    }

    fn map_openai_params(
        &self,
        image_edit_optional_params: &ImageParams,
        _model: &str,
        _drop_params: bool,
    ) -> ImageParams {
        map_openai_params(image_edit_optional_params)
    }

    fn transform_image_edit_request(
        &self,
        request: ImageEditRequest<'_>,
    ) -> Result<(ImageParams, ImageFiles), Self::Error> {
        let edit = OpenAiEditRequest {
            model: request.model.to_owned(),
            prompt: request.prompt.map(str::to_owned),
            image: Some(self.images.iter().cloned().map(Some).collect()),
            mask: self.mask.clone().map(|mask| vec![Some(mask)]),
            options: request.optional_params,
        };
        let form = transform_image_edit_request(self.family, &edit)?;
        Ok((form.data, form.files))
    }

    fn transform_image_edit_response(
        &self,
        reply: ImageEditReply<'_>,
    ) -> Result<ImageResponse, Self::Error> {
        let raw = reply.raw_response;
        let decoded = transform_image_edit_response(raw.status_code, &raw.body, 0)?;
        Ok(ImageResponse {
            created: decoded.created,
            data: Some(
                decoded
                    .data
                    .into_iter()
                    .map(|datum| ImageData {
                        b64_json: datum.b64_json,
                        revised_prompt: datum.revised_prompt,
                        url: datum.url,
                    })
                    .collect(),
            ),
            usage: None,
        })
    }

    // B512 sets Authorization from the resolved key over the caller's headers
    fn validate_environment(&self, request: ImageEditEnvironment<'_>) -> ImageHeaders {
        let credentials = OpenAiCredentials {
            api_base: request.api_base.unwrap_or_default().to_owned(),
            api_key: request
                .api_key
                .map(|key| SecretString::from(key.expose_secret())),
            organization: None,
        };
        // a header B512 refuses reaches the wire unchanged, where B2451 reports it
        let Ok(headers) = environment::validate_environment(request.headers, &credentials) else {
            return request.headers.clone();
        };
        headers
            .iter()
            .filter_map(|(name, value)| {
                let text = value.to_str().ok()?;
                Some((name.as_str().to_owned(), text.to_owned()))
            })
            .collect()
    }

    fn get_complete_url(
        &self,
        _model: &str,
        api_base: Option<&str>,
        _litellm_params: &ImageParams,
    ) -> Result<String, Self::Error> {
        let base = api_base.ok_or(ImageContractError::ApiBaseRequired)?;
        Ok(environment::complete_url(base)?.to_string())
    }
}
