use thiserror::Error;

use crate::count::error::CountError;
use crate::image::error::ImageError;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("the media content JSON could not be read")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("content block type {kind} is not counted")]
    UnsupportedBlock { kind: String },
    #[error("a nested content block type is not counted")]
    UnsupportedNestedBlock,
    #[error("a nested content list could not be read")]
    UnreadableNested,
    #[error("a string image_url must not be blank")]
    BlankImageUrl,
    #[error("an image_url object needs a url")]
    EmptyImageUrl,
    #[error("the {kind} source is not counted")]
    UnsupportedSource { kind: &'static str },
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
