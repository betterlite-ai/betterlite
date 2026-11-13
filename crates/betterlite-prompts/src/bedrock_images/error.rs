use betterlite_http::user_url::error::UserUrlError;
use thiserror::Error;

use crate::media_fetch::response::BodyReadError;
use crate::media_metadata::error::MediaMetadataError;

#[derive(Debug, Error)]
pub enum BedrockImageError {
    #[error("Unsupported image type. Expected either image url or base64 encoded string")]
    UnsupportedImageType,
    #[error("a base64 image URL needs exactly one comma between its metadata and its payload")]
    DataUrlParts,
    #[error("the base64 image's media type {mime_type:?} has no subtype")]
    MissingMediaSubtype { mime_type: String },
    #[error("the image format override {format:?} has no subtype")]
    MissingFormatSubtype { format: String },
    #[error("Unsupported image format: {image_format}. Supported formats: {supported}")]
    UnsupportedImageFormat {
        image_format: String,
        supported: &'static str,
    },
    #[error("No supported extensions for MIME type: {mime_type}. Supported formats: {supported}")]
    NoSupportedExtensions {
        mime_type: String,
        supported: &'static str,
    },
    #[error(transparent)]
    Metadata(#[from] MediaMetadataError),
    #[error(transparent)]
    Url(#[from] UserUrlError),
    #[error(transparent)]
    Body(#[from] BodyReadError),
    #[error(
        "{kind} '{status} {reason}' for url '{url}'\nFor more information check: https://developer.mozilla.org/en-US/docs/Web/HTTP/Status/{status}"
    )]
    HttpStatus {
        status: u16,
        kind: &'static str,
        reason: &'static str,
        url: String,
    },
    #[error("the image request did not finish within its deadline")]
    Deadline,
}
