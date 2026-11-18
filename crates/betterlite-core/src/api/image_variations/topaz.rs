use betterlite_openai::api::image_contracts::records::{ImageFile, ImageParams};
use secrecy::SecretString;

use super::PendingVariation;
use super::error::ImageVariationError;

// the aiohttp handler's call after main.image_variation resolved the provider and its credentials
#[derive(Debug, Clone)]
pub struct TopazVariationRequest {
    pub model: String,
    pub image: ImageFile,
    pub optional_params: ImageParams,
    pub api_key: SecretString,
    pub api_base: String,
    pub timeout_seconds: f64,
    pub async_call: bool,
    pub metadata: ImageParams,
}

// B046 supplies Topaz's transforms and aiohttp execution through this seam; Ok(None) is a response the
// branch did not produce, which main.image_variation turns into its invalid-provider ValueError.
pub trait TopazVariationExecutor {
    fn execute(
        &self,
        request: TopazVariationRequest,
    ) -> Result<Option<PendingVariation<'_>>, ImageVariationError>;
}
