use crate::image_generation::flux::is_flux2_model;
use crate::mai_images::url::is_mai_model;

use super::error::ImageEditError;

// get_azure_ai_image_edit_config checks MAI, then FLUX 2, then the FLUX 1 default branch
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageEditVariant {
    Flux1,
    Flux2,
    Mai,
}

impl ImageEditVariant {
    pub fn select(model: &str) -> Result<Self, ImageEditError> {
        if is_mai_model(model) {
            return Ok(Self::Mai);
        }
        if is_flux2_model(model) {
            return Ok(Self::Flux2);
        }
        let normalized = model.to_lowercase().replace(['-', '_'], "");
        if normalized.is_empty() || normalized.contains("flux") {
            return Ok(Self::Flux1);
        }
        Err(ImageEditError::UnsupportedModel {
            model: model.to_owned(),
        })
    }

    // the LiteLLM config class the variant stands for, so the oracle compares the branch rather than our name
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::Flux1 => "AzureFoundryFluxImageEditConfig",
            Self::Flux2 => "AzureFoundryFlux2ImageEditConfig",
            Self::Mai => "AzureFoundryMAIImageEditConfig",
        }
    }

    #[must_use]
    pub const fn uses_multipart(self) -> bool {
        match self {
            Self::Flux1 | Self::Mai => true,
            Self::Flux2 => false,
        }
    }
}
