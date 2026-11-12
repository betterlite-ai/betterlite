pub mod call_types;
pub mod dynamic_cache;
pub mod embedding;
pub mod kwargs;
pub mod response;
pub mod retrieve;
pub mod state;
pub mod value;

mod tests;

use thiserror::Error;

use crate::cache_key::error::CacheKeyError;

#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("the {field} value must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the cache key could not be derived")]
    Key {
        #[from]
        source: CacheKeyError,
    },
}
