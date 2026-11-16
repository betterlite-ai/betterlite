use betterlite_openai::image_generation::family::ImageFamily;

use super::target::is_mai_model;

// LiteLLM's Azure selector: the Azure classes only subclass the OpenAI ones except for MAI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureImageConfig {
    DallE2,
    DallE3,
    Mai,
    GptImage,
}

impl AzureImageConfig {
    #[must_use]
    pub const fn family(self) -> Option<ImageFamily> {
        match self {
            Self::DallE2 => Some(ImageFamily::DallE2),
            Self::DallE3 => Some(ImageFamily::DallE3),
            Self::GptImage => Some(ImageFamily::GptImage),
            Self::Mai => None,
        }
    }

    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::DallE2 => "AzureDallE2ImageGenerationConfig",
            Self::DallE3 => "AzureDallE3ImageGenerationConfig",
            Self::Mai => "AzureFoundryMAIImageGenerationConfig",
            Self::GptImage => "AzureGPTImageGenerationConfig",
        }
    }
}

#[must_use]
pub fn get_azure_image_generation_config(model: Option<&str>) -> AzureImageConfig {
    let original = model.unwrap_or_default();
    let normalized = original.to_lowercase().replace(['-', '_'], "");
    if normalized.is_empty() || normalized.contains("dalle2") {
        return AzureImageConfig::DallE2;
    }
    if normalized.contains("dalle3") {
        return AzureImageConfig::DallE3;
    }
    if is_mai_model(&normalized) {
        return AzureImageConfig::Mai;
    }
    tracing::debug!(
        "Using AzureGPTImageGenerationConfig for model: {original}. This follows the gpt-image model format."
    );
    AzureImageConfig::GptImage
}
