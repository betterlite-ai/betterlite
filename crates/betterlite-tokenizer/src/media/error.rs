use thiserror::Error;

use crate::count::error::CountError;
use crate::image::error::ImageError;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("the media content JSON could not be read or written")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("a content item must be a string or a content block")]
    ContentItem,
    #[error("a content block has no type")]
    MissingType,
    #[error("content block type {kind} is not counted")]
    UnsupportedBlock { kind: String },
    #[error("the {field} field of the {block} block is missing or has the wrong type")]
    Field {
        block: &'static str,
        field: &'static str,
    },
    #[error("the {kind} document source is not counted")]
    UnsupportedSource { kind: String },
    #[error("an image needs its exact dimensions and no local bytes were given")]
    RemoteNeedsDefaultCount,
    #[error("the content nesting exceeds {limit} levels")]
    TooDeep { limit: usize },
    #[error("the token count does not fit in a machine word")]
    Overflow,
    #[error("the image tokens could not be counted: {0}")]
    Image(#[from] ImageError),
    #[error("the text could not be counted: {0}")]
    Count(#[from] CountError),
}
