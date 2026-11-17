use std::convert::Infallible;

use super::error::VideoLifecycleError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsupportedOperation {
    Remix,
    List,
    CreateCharacter,
    GetCharacter,
    Edit,
    Extension,
}

impl UnsupportedOperation {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Remix => "Video remix is not yet supported by RunwayML API",
            Self::List => "Video listing is not yet supported by RunwayML API",
            Self::CreateCharacter => "video create character is not supported for RunwayML",
            Self::GetCharacter => "video get character is not supported for RunwayML",
            Self::Edit => "video edit is not supported for RunwayML",
            Self::Extension => "video extension is not supported for RunwayML",
        }
    }
}

// each request method raises its own NotImplementedError before any transport work begins
pub const fn unsupported_request(
    operation: UnsupportedOperation,
) -> Result<Infallible, VideoLifecycleError> {
    Err(VideoLifecycleError::Unsupported {
        message: operation.message(),
    })
}
