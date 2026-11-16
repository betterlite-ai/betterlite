use thiserror::Error;

use crate::gemini_http::error::GeminiHttpError;
use crate::gemini_request::error::RequestError;

// The facade fails where a dependency does: B1316 owns the HTTP error and B1321 owns the cache and body one
#[derive(Debug, Error)]
pub enum GeminiCallError {
    #[error(transparent)]
    Http(#[from] GeminiHttpError),
    #[error(transparent)]
    Request(#[from] RequestError),
}
