use serde_json::Error as JsonError;
use thiserror::Error;

use crate::bedrock_settings::{BedrockError, HeaderError};

#[derive(Debug, Error)]
pub enum CanvasEditError {
    #[error("Nova Canvas image edit requires an image input")]
    MissingImage,
    #[error(
        "Nova Canvas image edit does not support tuple FileTypes. Pass a file-like object, bytes, or a base64-encoded string."
    )]
    TupleFileType,
    #[error(
        "OUTPAINTING requires either a mask image or a mask prompt. Pass mask=<file> or maskPrompt=<str> in the request."
    )]
    OutPaintingMask,
    #[error(
        "Amazon Nova Canvas INPAINTING requires either maskPrompt or maskImage (use OpenAI mask= for maskImage, or pass maskPrompt in optional params). See https://docs.aws.amazon.com/nova/latest/userguide/image-gen-req-resp-structure.html"
    )]
    InPaintingMask,
    #[error(
        "Unsupported Amazon Nova Canvas taskType: '{task_type}'. Use BACKGROUND_REMOVAL, OUTPAINTING, IMAGE_VARIATION, INPAINTING, or omit taskType for automatic routing (mask → INPAINTING, else IMAGE_VARIATION)."
    )]
    UnsupportedTaskType { task_type: String },
    #[error(
        "Amazon Nova Canvas {task_type} requires a text prompt. Pass a non-empty `prompt` in your request."
    )]
    MissingPrompt { task_type: String },
    #[error("the {field} value has a type the Nova Canvas transform does not declare")]
    Field {
        field: String,
        #[source]
        source: JsonError,
    },
    #[error(
        "Unsupported Bedrock image-edit model: '{model}'. Use a stability.* image-edit model id or add supports_nova_canvas_image_edit in model_prices for this id."
    )]
    UnsupportedModel { model: String },
    #[error(
        "Nova Canvas image edit URLs are built in BedrockImageEdit._prepare_request (AWS runtime endpoint + model invoke path). Do not use get_complete_url for this config."
    )]
    CompleteUrl,
    #[error("bedrock error ({status})")]
    Provider {
        status: u16,
        error: Box<BedrockError>,
    },
    #[error(transparent)]
    Header(#[from] HeaderError),
}

impl CanvasEditError {
    // The status LiteLLM's BedrockError carries; None is what a Python ValueError reports
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Provider { status, .. } => Some(*status),
            Self::MissingImage
            | Self::TupleFileType
            | Self::OutPaintingMask
            | Self::InPaintingMask
            | Self::UnsupportedTaskType { .. }
            | Self::MissingPrompt { .. }
            | Self::Field { .. }
            | Self::UnsupportedModel { .. }
            | Self::CompleteUrl
            | Self::Header(_) => None,
        }
    }
}
