use betterlite_openai::api::file_type_records::FileTypeError;
use thiserror::Error;
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
    // a bodyless BadRequestError leaves type null, like betterlite-error's BadRequest shape
    #[must_use]
    pub const fn error_type(&self) -> Option<&'static str> {
        if let Self::UnsupportedMediaType(..) = self {
            return None;
        }
        Some("invalid_request_error")
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
        match self {
            Self::UnsupportedMediaType(..) => Some("400"),
            Self::RemoteMediaRequired(..) => Some("unsupported_remote_media"),
            Self::InvalidMedia(..) | Self::InvalidInlineData(..) => None,
        }
    }
}
