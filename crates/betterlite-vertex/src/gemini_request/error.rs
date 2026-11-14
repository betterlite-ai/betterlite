use betterlite_prompts::error::FactoryError;
use betterlite_prompts::media::error::MediaError;
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::cache_create::error::CacheCreateError;
use crate::gemini_history::error::HistoryError;

#[derive(Debug, Error)]
pub enum BodyError {
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Prompt(#[from] FactoryError),
    #[error("`{0}` must be {1}")]
    Field(&'static str, &'static str),
    #[error("a request body value could not be converted")]
    Encode(#[source] serde_json::Error),
}

// the cache request and the body build fail on their own terms; the caller maps either
#[derive(Debug, Error)]
pub enum RequestError {
    #[error(transparent)]
    Cache(#[from] CacheCreateError),
    #[error(transparent)]
    Body(#[from] BodyError),
    #[error(transparent)]
    Inline(#[from] MediaError),
}

pub fn encode<T, U>(value: &T) -> Result<U, BodyError>
where
    T: Serialize + ?Sized,
    U: DeserializeOwned,
{
    serde_json::from_value(serde_json::to_value(value).map_err(BodyError::Encode)?)
        .map_err(BodyError::Encode)
}
