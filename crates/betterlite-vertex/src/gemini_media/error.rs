use betterlite_openai::api::file_type_records::FileTypeError;
use thiserror::Error;
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MediaError {
    #[error("file type not supported by gemini: {0}")]
    UnsupportedMediaType(String, #[source] Option<FileTypeError>),
    // the param names the media field the caller supplied, image_url or file
    #[error("this media needs a remote fetch")]
    RemoteMediaRequired(&'static str),
    #[error("invalid media reference: {0}")]
    InvalidMedia(String),
    #[error("malformed inline media data: {0}")]
    InvalidInlineData(String),
}
