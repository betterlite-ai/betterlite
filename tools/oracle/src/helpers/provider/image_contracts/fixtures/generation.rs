use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_provider::api::image_contracts::error::ImageContractError;
use betterlite_provider::api::image_contracts::generation::BaseImageGenerationConfig;

pub(super) struct GenerationFixture;

impl BaseImageGenerationConfig for GenerationFixture {
    type Error = ImageContractError;

    fn get_supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_openai_params(
        &self,
        _non_default_params: &ImageParams,
        optional_params: ImageParams,
        _model: &str,
        _drop_params: bool,
    ) -> ImageParams {
        optional_params
    }
}
