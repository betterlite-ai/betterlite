use betterlite_http::error::HttpError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VideoEditError {
    #[error("the character id cannot become a path segment")]
    CharacterId(#[from] HttpError),
    #[error("the character reply is not a character object")]
    CharacterReply(#[source] serde_json::Error),
    #[error("the video reply is not a video object")]
    VideoReply(#[source] serde_json::Error),
}
