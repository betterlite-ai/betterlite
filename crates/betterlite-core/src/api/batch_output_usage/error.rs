use std::convert::Infallible;

use betterlite_anthropic::usage::error::UsageError as AnthropicUsageError;
use betterlite_logging::fields::usage::UsageError as LoggingUsageError;
use betterlite_models::ocr_pricing::OcrCostError;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_models::usage_sum::UsageSumError;
use betterlite_tokenizer::count::error::CountError;
use thiserror::Error;

use crate::api::responses_usage::ResponsesUsageError;
use crate::cost::batch::error::BatchCostError;
use crate::cost::completion::usage::UsageConversionError;

pub type BatchResult<T> = Result<T, BatchOutputUsageError>;

// One bad line is billed at $0 by the aggregate; only the file-level variants abort the batch.
#[derive(Debug, Error)]
pub enum BatchOutputUsageError {
    #[error("the batch output is {0} bytes; the maximum is {1}")]
    JsonTooLarge(usize, usize),
    #[error("a batch output line is {0} bytes; the maximum is {1}")]
    LineTooLarge(usize, usize),
    #[error("the batch output has more than {0} lines")]
    TooManyLines(usize),
    #[error("the batch output overflowed at {field}")]
    Overflow { field: &'static str },
    #[error("the batch output usage totals could not be summed: {0}")]
    UsageSum(#[from] UsageSumError),
    #[error("the {field} field must be an object")]
    NotAnObject { field: &'static str },
    #[error("the {field} field must be an integer")]
    NotAnInteger { field: &'static str },
    #[error("the {field} field must be a number")]
    NotANumber { field: &'static str },
    #[error("the {field} field must be a string")]
    NotAString { field: &'static str },
    #[error("the batch input entry must be an object")]
    NotAnEntry,
    #[error("the provider usage object could not be read: {0}")]
    AnthropicUsage(#[from] AnthropicUsageError<Infallible>),
    #[error("the provider usage object could not be converted: {0}")]
    LoggingUsage(#[from] LoggingUsageError),
    #[error("the provider usage object could not be priced: {0}")]
    Pricing(#[from] UsageConversionError),
    #[error("the responses usage object could not be read: {0}")]
    ResponsesUsage(#[from] ResponsesUsageError),
    #[error("the batch cost could not be computed: {0}")]
    BatchCost(#[from] BatchCostError),
    #[error("the prompt token detail could not be computed: {0}")]
    TokenCost(#[from] TokenCostError),
    #[error("the OCR batch cost could not be computed: {0}")]
    OcrCost(#[from] OcrCostError),
    #[error("the provider payload could not be read as JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the messages field could not be read as chat messages: {source}")]
    Messages { source: serde_json::Error },
    #[error("the token count failed: {0}")]
    Count(#[from] CountError),
    #[error("the converse usage object matches the converse shape but cannot be read")]
    ConverseUsage,
}
