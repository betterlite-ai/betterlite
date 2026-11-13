use std::error::Error as StdError;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConverseUsageError {
    #[error("the Bedrock usage block could not be read")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("a Bedrock reasoning content block is not an object")]
    BlockNotObject,
    #[error("the field {field} has a type the Converse usage shape does not allow")]
    FieldType { field: &'static str },
    #[error("the field {field} is missing from the Converse usage block")]
    Missing { field: &'static str },
    #[error("the field {field} is outside the signed 64-bit range")]
    OutOfRange { field: &'static str },
    #[error("the arithmetic at {field} overflowed")]
    Overflow { field: &'static str },
    #[error("a reasoningText block has no text")]
    ReasoningTextMissing,
}

#[derive(Debug, Error)]
pub enum UsageError<CounterFailure>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Usage(#[from] ConverseUsageError),
    #[error("the reasoning token counter failed")]
    Counter {
        #[source]
        source: CounterFailure,
    },
}
