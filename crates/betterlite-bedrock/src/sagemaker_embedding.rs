use betterlite_openai::openai_base_config::Headers;

pub mod cohere;
pub mod hf;

// VoyageEmbeddingConfig.get_supported_openai_params: the list the Sagemaker factory returns for a Voyage model
pub const VOYAGE_SUPPORTED_OPENAI_PARAMS: &[&str] = &["encoding_format", "dimensions"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingConfig {
    Voyage,
    Cohere,
    HuggingFace,
}

// get_model_config: the voyage check runs first, so a cohere-voyage model is a Voyage one
#[must_use]
pub fn get_model_config(model: &str) -> EmbeddingConfig {
    let lowered = model.to_lowercase();
    if lowered.contains("voyage") {
        EmbeddingConfig::Voyage
    } else if lowered.contains("cohere") {
        EmbeddingConfig::Cohere
    } else {
        EmbeddingConfig::HuggingFace
    }
}

#[must_use]
pub fn supported_openai_params(model: &str) -> &'static [&'static str] {
    match get_model_config(model) {
        EmbeddingConfig::Voyage => VOYAGE_SUPPORTED_OPENAI_PARAMS,
        EmbeddingConfig::Cohere => cohere::SUPPORTED_OPENAI_PARAMS,
        EmbeddingConfig::HuggingFace => &[],
    }
}

// both validate_environment methods drop the caller's headers and keep only the content type
#[must_use]
pub fn validate_environment() -> Headers {
    Headers::from([(
        String::from("Content-Type"),
        String::from("application/json"),
    )])
}
mod tests;
