use std::str::Utf8Error;

use thiserror::Error;

use crate::gemini_tools::error::GeminiToolsError;

#[derive(Debug, Error)]
pub enum GeminiImagePolicyError {
    #[error("`imageConfig` must be valid JSON when provided as a string.")]
    ImageConfigJson(#[source] serde_json::Error),
    #[error("mapping the image tools failed")]
    Tools(#[from] GeminiToolsError),
    #[error("encoding a mapped image tool failed")]
    Encode(#[source] serde_json::Error),
    #[error("tools must be a list of objects")]
    ToolsShape,
    #[error("image options exceed the {limit}-byte limit")]
    OptionsTooLarge { limit: usize },
    #[error("image options are not UTF-8")]
    OptionsNotUtf8(#[source] Utf8Error),
    #[error("image options are not valid JSON")]
    OptionsJson(#[source] serde_json::Error),
    #[error("image options must be a size string or a JSON object")]
    OptionsShape,
}
