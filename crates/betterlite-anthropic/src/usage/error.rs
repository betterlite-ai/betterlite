use std::error::Error as StdError;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum UsageError<CounterFailure>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[error("the Anthropic usage object could not be read")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the usage field {field} has a type the declared Anthropic shape does not allow")]
    FieldType { field: &'static str },
    #[error("the usage field {field} is outside the signed 64-bit range")]
    OutOfRange { field: &'static str },
    #[error("the usage arithmetic overflowed at {field}")]
    Overflow { field: &'static str },
    #[error("the reasoning token counter failed")]
    Counter {
        #[source]
        source: CounterFailure,
    },
}
