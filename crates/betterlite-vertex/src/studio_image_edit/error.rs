use std::io;

use http::header::HeaderMap;
use thiserror::Error;

use crate::gemini_image_policy::error::GeminiImagePolicyError;
use crate::studio_image_generation::error::StudioImageError;
use crate::vertex_gemini_images::error::GeminiImageError;

// an imageConfig string that is not JSON is LiteLLM's UnsupportedParamsError, which carries 400
const POLICY_STATUS: u16 = 400;

#[derive(Debug, Error)]
pub enum StudioImageEditError {
    // LiteLLM raises ValueError when no image part survives preparation
    #[error("the Gemini image edit request carries no image")]
    MissingImage,
    #[error("an edit image must be bytes or a seekable stream")]
    UnsupportedImage,
    #[error("the image stream could not be read")]
    Stream(#[source] io::Error),
    #[error(transparent)]
    Reply(#[from] GeminiImageError),
    // the usage conversion is B1350's, so its own failure rides along unchanged
    #[error(transparent)]
    Usage(#[from] StudioImageError),
    #[error(transparent)]
    Policy(#[from] GeminiImagePolicyError),
}

impl StudioImageEditError {
    // only a reply body that does not parse keeps the reply's status and headers
    #[must_use]
    pub fn upstream_answer(&self) -> Option<(u16, &HeaderMap)> {
        match self {
            Self::Reply(error) => error.upstream_answer(),
            Self::MissingImage
            | Self::UnsupportedImage
            | Self::Stream(_)
            | Self::Usage(_)
            | Self::Policy(_) => None,
        }
    }

    // map_openai_params rejects an imageConfig string that is not JSON before any request is sent
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Policy(GeminiImagePolicyError::ImageConfigJson(_)) => Some(POLICY_STATUS),
            Self::MissingImage
            | Self::UnsupportedImage
            | Self::Stream(_)
            | Self::Reply(_)
            | Self::Usage(_)
            | Self::Policy(_) => None,
        }
    }
}
