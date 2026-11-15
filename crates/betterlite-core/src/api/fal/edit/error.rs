use std::io;
use std::path::PathBuf;

use thiserror::Error;

use crate::api::fal::images::error::FalImageError;

#[derive(Debug, Error)]
pub enum FalEditError {
    #[error("Unsupported image type for Fal AI image edit: {kind}")]
    UnsupportedImage { kind: &'static str },
    #[error("Fal AI image edit requires at least one input image")]
    MissingImages,
    #[error("{endpoint} accepts exactly one control image")]
    ControlImages { endpoint: String },
    #[error("failed to read the image file {path}: {source}")]
    ReadPath { path: PathBuf, source: io::Error },
    #[error("the image reader failed: {source}")]
    Reader { source: io::Error },
    #[error("Error parsing Fal AI image edit response: {source}")]
    Reply {
        #[from]
        source: FalImageError,
    },
}
