use std::num::ParseIntError;

use betterlite_http::error::HttpError;
use betterlite_prompts::media_metadata::error::MediaMetadataError;
use thiserror::Error;

use crate::provider_metadata::timestamp::VertexTimestampError;

#[derive(Debug, Error)]
pub enum StudioFilesError {
    #[error("File data is required")]
    FileDataRequired,
    #[error("GEMINI_API_KEY is required for Google AI Studio file operations")]
    MissingSettingsKey,
    #[error("api_key is required")]
    MissingApiKey,
    #[error(transparent)]
    FileId(#[from] HttpError),
    #[error(transparent)]
    Media(#[from] MediaMetadataError),
    #[error("the file reply is {size} bytes, over the {limit} byte limit")]
    ResponseTooLarge { size: usize, limit: usize },
    #[error("the file reply is not a Google AI Studio file record")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the file upload plan is not JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the file reply sizeBytes is not an integer")]
    SizeBytes {
        #[source]
        source: ParseIntError,
    },
    #[error("the file timestamp is {size} bytes, over the {limit} byte limit")]
    TimestampTooLong { size: usize, limit: usize },
    #[error("the file timestamp does not match the %Y-%m-%dT%H:%M:%S.%f%z format")]
    TimestampFormat,
    #[error("the file timestamp names an invalid date or time")]
    Timestamp {
        #[source]
        source: VertexTimestampError,
    },
    #[error("Failed to delete file: {body}")]
    DeleteFailed { status: u16, body: String },
    #[error("{operation} is not supported by the Google AI Studio files API")]
    Unsupported { operation: &'static str },
}
