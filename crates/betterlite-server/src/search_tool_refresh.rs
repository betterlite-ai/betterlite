use std::error::Error as StdError;
use std::future::Future;

use thiserror::Error;

pub trait RouterSearchRefresh: Send + Sync {
    fn refresh(&self) -> impl Future<Output = Result<(), SearchRefreshError>> + Send;
}

#[derive(Debug, Error)]
#[error("{source}")]
pub struct SearchRefreshError {
    // the reload lives in the binary crate, whose error type this crate cannot name
    #[source]
    pub source: Box<dyn StdError + Send + Sync>,
}

mod tests;
