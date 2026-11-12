use async_trait::async_trait;
use betterlite_openai::api::image_contracts::records::{ImageRequestFields, ImageResponse};

use crate::api::image_contracts::error::ImageContractError;
use crate::api::image_contracts::variation::{
    BaseImageVariationConfig, ImageVariationReply, ImageVariationRequest,
};

use super::Fixture;

#[async_trait]
impl BaseImageVariationConfig for Fixture {
    type Error = ImageContractError;

    fn get_supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn transform_request_image_variation(
        &self,
        _request: ImageVariationRequest<'_>,
    ) -> Result<ImageRequestFields, Self::Error> {
        Ok(ImageRequestFields::default())
    }

    async fn async_transform_response_image_variation(
        &self,
        _reply: ImageVariationReply<'_>,
    ) -> Result<ImageResponse, ImageContractError> {
        Ok(ImageResponse::default())
    }

    fn transform_response_image_variation(
        &self,
        _reply: ImageVariationReply<'_>,
    ) -> Result<ImageResponse, Self::Error> {
        Ok(ImageResponse::default())
    }
}
