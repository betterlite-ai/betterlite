use crate::mai_images::url::is_mai_model;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoundryImageConfig {
    DallE2,
    DallE3,
    Mai,
    Flux,
    GptImage,
}

impl FoundryImageConfig {
    // LiteLLM lowercases and strips hyphens and underscores first, and each earlier marker wins over every later one
    #[must_use]
    pub fn select(model: &str) -> Self {
        let normalized = model.to_lowercase().replace(['-', '_'], "");
        if normalized.is_empty() || normalized.contains("dalle2") {
            Self::DallE2
        } else if normalized.contains("dalle3") {
            Self::DallE3
        } else if is_mai_model(&normalized) {
            Self::Mai
        } else if normalized.contains("flux") {
            Self::Flux
        } else {
            tracing::debug!(model = model, "using the GPT image generation config");
            Self::GptImage
        }
    }
}
