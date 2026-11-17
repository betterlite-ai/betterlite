use betterlite_openai::image_generation::reply::ImageReplyError;
use betterlite_openai::openai_base_config::forwarding::BaseConfigError;
use thiserror::Error;

use crate::credentials::error::CredentialsError;
use crate::image_generation::error::FoundryImageError;
use crate::mai_images::error::MaiImageError;

#[derive(Debug, Error)]
pub enum ImageEditError {
    #[error("Model {model} is not supported for Azure AI image editing.")]
    UnsupportedModel { model: String },
    #[error(
        "Azure AI API base is required. Set AZURE_AI_API_BASE environment variable or pass api_base parameter."
    )]
    ApiBaseRequired,
    #[error(
        "Azure API version is required. Set AZURE_AI_API_VERSION environment variable or pass api_version parameter."
    )]
    ApiVersionRequired,
    #[error("FLUX 2 image edit requires a prompt.")]
    Flux2PromptRequired,
    #[error("FLUX 2 image edit requires an image.")]
    Flux2ImageRequired,
    #[error("FLUX 2 image edit requires at least one image.")]
    Flux2NoReferenceImages,
    #[error("{model} supports at most {limit} reference images.")]
    Flux2TooManyReferenceImages { model: String, limit: usize },
    #[error("Unsupported image type: {kind}")]
    UnsupportedImageType { kind: String },
    #[error(transparent)]
    Url(#[from] BaseConfigError),
    #[error(transparent)]
    Params(#[from] FoundryImageError),
    #[error(transparent)]
    Credentials(#[from] CredentialsError),
    #[error(transparent)]
    Mai(#[from] MaiImageError),
    #[error(transparent)]
    Projection(#[from] ImageReplyError),
    #[error("the MAI image edit reply could not be re-encoded")]
    ReplyEncoding(#[source] serde_json::Error),
}

enum ProviderFailure<'error> {
    Params(&'error FoundryImageError),
    Mai(&'error MaiImageError),
    Status(u16),
}

impl ImageEditError {
    // FLUX raises ValueError with its own 400s, and a non-JSON MAI reply keeps the provider status through B668
    fn provider_failure(&self) -> Option<ProviderFailure<'_>> {
        match self {
            Self::Params(error) => Some(ProviderFailure::Params(error)),
            Self::Mai(error) => Some(ProviderFailure::Mai(error)),
            Self::Projection(ImageReplyError::ResponseObject { status, .. }) => {
                Some(ProviderFailure::Status(*status))
            }
            Self::UnsupportedModel { .. }
            | Self::ApiBaseRequired
            | Self::ApiVersionRequired
            | Self::Flux2PromptRequired
            | Self::Flux2ImageRequired
            | Self::Flux2NoReferenceImages
            | Self::Flux2TooManyReferenceImages { .. }
            | Self::UnsupportedImageType { .. }
            | Self::Url(_)
            | Self::Credentials(_)
            | Self::Projection(_)
            | Self::ReplyEncoding(_) => None,
        }
    }

    #[must_use]
    pub fn status_code(&self) -> Option<u16> {
        match self.provider_failure()? {
            ProviderFailure::Params(error) => error.status_code(),
            ProviderFailure::Mai(error) => error.status_code(),
            ProviderFailure::Status(status) => Some(status),
        }
    }

    #[must_use]
    pub fn llm_provider(&self) -> Option<&'static str> {
        match self.provider_failure()? {
            ProviderFailure::Params(error) => error.llm_provider(),
            ProviderFailure::Mai(error) => error.llm_provider(),
            ProviderFailure::Status(_) => None,
        }
    }
}
