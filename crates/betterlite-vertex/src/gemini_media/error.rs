use betterlite_openai::api::file_type_records::FileTypeError;
use thiserror::Error;
const INVALID_REQUEST: &str = "invalid_request_error";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaField {
    ImageUrl,
    File,
}
impl MediaField {
    #[must_use]
    pub const fn param(self) -> &'static str {
        match self {
            Self::ImageUrl => "image_url",
            Self::File => "file",
        }
    }
}
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MediaError {
    #[error("file type not supported by gemini: {0}")]
    UnsupportedMediaType(String, #[source] Option<FileTypeError>),
    #[error("this media needs a remote fetch")]
    RemoteMediaRequired(MediaField),
    #[error("invalid media reference: {0}")]
    InvalidMedia(String),
    #[error("malformed inline media data: {0}")]
    InvalidInlineData(String),
}
impl MediaError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::UnsupportedMediaType(..)
            | Self::RemoteMediaRequired(..)
            | Self::InvalidMedia(..)
            | Self::InvalidInlineData(..) => 400,
        }
    }
    #[must_use]
    pub const fn error_type(&self) -> &'static str {
        match self {
            Self::UnsupportedMediaType(..)
            | Self::RemoteMediaRequired(..)
            | Self::InvalidMedia(..)
            | Self::InvalidInlineData(..) => INVALID_REQUEST,
        }
    }
    #[must_use]
    pub const fn param(&self) -> Option<&'static str> {
        if let Self::RemoteMediaRequired(field) = self {
            return Some(field.param());
        }
        None
    }
    #[must_use]
    pub const fn code(&self) -> Option<&'static str> {
        if self.param().is_some() {
            return Some("unsupported_remote_media");
        }
        None
    }
}
