use thiserror::Error;

use betterlite_openai::api::file_type_records::FileTypeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaField {
    ImageUrl,
    File,
}

impl MediaField {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ImageUrl => "image_url",
            Self::File => "file",
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MediaError {
    #[error("file type not supported by gemini: {subject}")]
    UnsupportedMediaType {
        subject: String,
        #[source]
        source: Option<FileTypeError>,
    },
    #[error("media needs a remote fetch this gateway does not make: {url}")]
    RemoteMediaRequired { url: String, field: MediaField },
    #[error("invalid media reference: {value}")]
    InvalidMedia { value: String },
    #[error("malformed inline media data: {value}")]
    InvalidInlineData { value: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaFailure {
    pub status: u16,
    pub error_type: &'static str,
    pub param: Option<&'static str>,
    pub code: Option<&'static str>,
}

impl MediaError {
    #[must_use]
    pub const fn failure(&self) -> MediaFailure {
        match self {
            Self::UnsupportedMediaType { .. }
            | Self::InvalidMedia { .. }
            | Self::InvalidInlineData { .. } => MediaFailure {
                status: 400,
                error_type: "invalid_request_error",
                param: None,
                code: None,
            },
            Self::RemoteMediaRequired { field, .. } => MediaFailure {
                status: 400,
                error_type: "invalid_request_error",
                param: Some(field.name()),
                code: Some("unsupported_remote_media"),
            },
        }
    }
}
