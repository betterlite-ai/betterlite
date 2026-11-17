use thiserror::Error;

// every request method fal.ai's config implements, each of which only reports that the provider has no endpoint
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum UnsupportedOperation {
    #[error("video remix is not supported for fal.ai")]
    Remix,
    #[error("video listing is not supported for fal.ai")]
    List,
    #[error("video delete is not supported for fal.ai")]
    Delete,
    #[error("video character creation is not supported for fal.ai")]
    CreateCharacter,
    #[error("video character retrieval is not supported for fal.ai")]
    GetCharacter,
    #[error("video edit is not supported for fal.ai")]
    Edit,
    #[error("video extension is not supported for fal.ai")]
    Extension,
}

pub const ALL: [UnsupportedOperation; 7] = [
    UnsupportedOperation::Remix,
    UnsupportedOperation::List,
    UnsupportedOperation::Delete,
    UnsupportedOperation::CreateCharacter,
    UnsupportedOperation::GetCharacter,
    UnsupportedOperation::Edit,
    UnsupportedOperation::Extension,
];

impl UnsupportedOperation {
    // the config method LiteLLM raises from, by the part of its name after transform_video_
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Remix => "remix",
            Self::List => "list",
            Self::Delete => "delete",
            Self::CreateCharacter => "character_create",
            Self::GetCharacter => "character_get",
            Self::Edit => "edit",
            Self::Extension => "extension",
        }
    }
}
