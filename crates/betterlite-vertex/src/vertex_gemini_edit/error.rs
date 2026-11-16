use std::io;

use http::header::HeaderMap;
use thiserror::Error;

use crate::vertex_gemini_images::error::GeminiImageError;

#[derive(Debug, Error)]
pub enum GeminiEditError {
    #[error("`{param}` must be {expected}")]
    FieldType {
        param: &'static str,
        expected: &'static str,
    },
    // LiteLLM raises ValueError when the prepared parts come out empty
    #[error("Vertex AI Gemini image edit requires at least one image")]
    MissingImage,
    // LiteLLM's reader rejects every record that is not bytes, BytesIO or BufferedReader
    #[error("the image record is not bytes or a seekable stream")]
    UnsupportedImage,
    #[error("reading the image stream failed")]
    Stream(#[source] io::Error),
    // the reply decoder is shared with generation, so its own error keeps the upstream answer
    #[error(transparent)]
    Reply(#[from] GeminiImageError),
}

impl GeminiEditError {
    // only a reply body that does not parse keeps the reply's status and headers
    #[must_use]
    pub fn upstream_answer(&self) -> Option<(u16, &HeaderMap)> {
        match self {
            Self::Reply(error) => error.upstream_answer(),
            Self::FieldType { .. }
            | Self::MissingImage
            | Self::UnsupportedImage
            | Self::Stream(_) => None,
        }
    }
}
