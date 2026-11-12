use async_trait::async_trait;
use betterlite_openai::api::image_contracts::records::{ImageRequestFields, ImageResponse};
use betterlite_provider::api::image_contracts::error::ImageContractError;
use betterlite_provider::api::image_contracts::variation::{
    BaseImageVariationConfig, ImageVariationReply, ImageVariationRequest,
};

pub(super) struct VariationFixture;

#[async_trait]
impl BaseImageVariationConfig for VariationFixture {
    type Error = ImageContractError;

    fn get_supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    // ast-grep-ignore: no-long-names
    fn transform_request_image_variation(
        &self,
        _request: ImageVariationRequest<'_>,
    ) -> Result<ImageRequestFields, Self::Error> {
        Ok(ImageRequestFields::default())
    }

    // ast-grep-ignore: no-long-names
    async fn async_transform_response_image_variation(
        &self,
        _reply: ImageVariationReply<'_>,
    ) -> Result<ImageResponse, ImageContractError> {
        Ok(ImageResponse::default())
    }

    // ast-grep-ignore: no-long-names
    fn transform_response_image_variation(
        &self,
        _reply: ImageVariationReply<'_>,
    ) -> Result<ImageResponse, Self::Error> {
        Ok(ImageResponse::default())
    }
}
