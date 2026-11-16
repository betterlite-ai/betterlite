use std::io;

use betterlite_prompts::media::data_url::MAX_MEDIA_BYTES;
use betterlite_streaming::stream_content_assembly::AssemblyError;
use http::header::HeaderMap;
use serde_json::Error as JsonError;
use thiserror::Error;

use crate::vertex_image_identity::error::ImageIdentityError;
use crate::vertex_imagen_generation::error::ImagenGenerationError;

#[derive(Debug, Error)]
pub enum ImagenEditError {
    // LiteLLM raises one ValueError for an absent, empty or all-None reference list
    #[error("Vertex AI Imagen image edit requires at least one reference image")]
    MissingImage,
    // the prompt check runs after every reference was read, and an empty prompt is allowed
    #[error("Vertex AI Imagen image edit requires a prompt")]
    MissingPrompt,
    #[error("`{param}` must be {expected}")]
    FieldType {
        param: &'static str,
        expected: &'static str,
    },
    // a bare str record is refused instead of being decoded as base64, as LiteLLM's reader does
    #[error(
        "the image record is a plain string, and only bytes or a file-like object are accepted"
    )]
    PlainString,
    #[error(
        "the image record is a filesystem path, and this adapter reads bytes or a file-like object"
    )]
    FilePath,
    #[error("the image record nests deeper than the {max_depth} level limit")]
    Depth { max_depth: i64 },
    #[error("the image record is {kind} and holds no image bytes")]
    UnsupportedImage { kind: &'static str },
    // the streaming decoder holds CPython's permissive reading of base64, junk and padding included
    #[error("the image record's base64 text cannot be decoded")]
    Base64(#[source] AssemblyError),
    #[error("the image input is {size} bytes, over the {MAX_MEDIA_BYTES} byte ceiling")]
    TooLarge { size: usize },
    #[error("reading the image stream failed")]
    Stream(#[source] io::Error),
    #[error("the image input is not a JSON document")]
    InputJson(#[source] JsonError),
    #[error("the Imagen edit request could not be serialized")]
    Serialize(#[source] JsonError),
    // the identity resolver keeps its own error, whose missing-target message is LiteLLM's
    #[error(transparent)]
    Target(#[from] ImageIdentityError),
    // the reply decoder is shared with generation, so its own error keeps the upstream answer
    #[error(transparent)]
    Reply(#[from] ImagenGenerationError),
}

impl ImagenEditError {
    // only a reply body that does not parse keeps the reply's status and headers
    #[must_use]
    pub fn upstream_answer(&self) -> Option<(u16, &HeaderMap)> {
        match self {
            Self::Reply(error) => error.upstream_answer(),
            Self::MissingImage
            | Self::MissingPrompt
            | Self::FieldType { .. }
            | Self::PlainString
            | Self::FilePath
            | Self::Depth { .. }
            | Self::UnsupportedImage { .. }
            | Self::Base64(_)
            | Self::TooLarge { .. }
            | Self::Stream(_)
            | Self::InputJson(_)
            | Self::Serialize(_)
            | Self::Target(_) => None,
        }
    }
}
