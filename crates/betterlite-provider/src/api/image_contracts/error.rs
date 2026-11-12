use thiserror::Error;

// LiteLLM's image generation and variation request stubs both name ImageVariationConfig, typo included
pub const NOT_IMPLEMENTED_REQUEST: &str = "ImageVariationConfig implementa 'transform_request_image_variation' for image variation models";
pub const NOT_IMPLEMENTED_RESPONSE: &str = "ImageVariationConfig implements 'transform_response_image_variation' for image variation models";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ImageContractError {
    #[error("api_base is required")]
    ApiBaseRequired,
    #[error("{message}")]
    NotImplemented { message: &'static str },
}
