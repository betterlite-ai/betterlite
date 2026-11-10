use betterlite_openai::api::file_type_records::FileTypeError;
use thiserror::Error;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaField {
    ImageUrl,
    File,
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
            | Self::InvalidInlineData(..) => "invalid_request_error",
        }
    }
    #[must_use]
    pub const fn param(&self) -> Option<&'static str> {
        match self {
            Self::RemoteMediaRequired(MediaField::ImageUrl) => Some("image_url"),
            Self::RemoteMediaRequired(MediaField::File) => Some("file"),
            Self::UnsupportedMediaType(..)
            | Self::InvalidMedia(..)
            | Self::InvalidInlineData(..) => None,
        }
    }
    #[must_use]
    pub const fn code(&self) -> Option<&'static str> {
        match self {
            Self::RemoteMediaRequired(..) => Some("unsupported_remote_media"),
            Self::UnsupportedMediaType(..)
            | Self::InvalidMedia(..)
            | Self::InvalidInlineData(..) => None,
        }
    }
}
