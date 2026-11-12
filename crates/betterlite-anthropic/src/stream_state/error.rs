use std::convert::Infallible;
use std::error::Error as StdError;

use thiserror::Error;

use crate::usage::error::UsageError;

#[derive(Debug, Error)]
pub enum StreamStateError<CounterFailure = Infallible>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[error("the Anthropic stream event could not be decoded")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "the Anthropic stream event field {field} has a type its declared shape does not allow"
    )]
    FieldType { field: &'static str },
    #[error("the Anthropic stream usage could not be calculated")]
    Usage {
        #[from]
        source: UsageError<CounterFailure>,
    },
}
