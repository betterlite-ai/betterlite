use betterlite_anthropic::usage::error::UsageError as AnthropicUsageError;
use betterlite_logging::fields::usage::UsageError as LoggingUsageError;

use super::projection::NoReasoningContent;

#[derive(Debug, thiserror::Error)]
pub(crate) enum StreamCostError {
    #[error("the cost frame is not a JSON object")]
    NotAnObject,
    #[error("the stream usage field {field} must be an object")]
    DetailsNotAnObject { field: &'static str },
    #[error("the stream usage field {field} must be an integer")]
    NotAnInteger { field: &'static str },
    #[error("the stream usage field {field} must be a number")]
    NotANumber { field: &'static str },
    #[error("the token count {field} does not fit a signed 64-bit integer")]
    Overflow { field: &'static str },
    #[error("the Anthropic usage could not be calculated")]
    Anthropic {
        #[source]
        source: AnthropicUsageError<NoReasoningContent>,
    },
    #[error("the stream usage could not be read")]
    Usage {
        #[source]
        source: LoggingUsageError,
    },
    #[error("the stream usage field {field} could not be converted")]
    Convert {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the calculated usage is not a JSON object")]
    UsageNotAnObject,
}
