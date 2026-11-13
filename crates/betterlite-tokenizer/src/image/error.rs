use betterlite_prompts::media::error::MediaError;
use thiserror::Error;

use crate::image::header::ImageType;

#[derive(Debug, Error)]
pub enum ImageError {
    #[error("the image data URL could not be decoded")]
    Media(#[from] MediaError),
    #[error("the {kind:?} header ends before its dimension fields")]
    Truncated { kind: ImageType },
    #[error("a zero image side cannot be resized")]
    ZeroSide,
    #[error("a zero tile size cannot divide an image")]
    ZeroTile,
    #[error("the image token arithmetic overflowed")]
    Overflow,
}
