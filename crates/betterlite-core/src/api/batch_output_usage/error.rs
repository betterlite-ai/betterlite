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

#[derive(Debug, Error)]
pub enum BatchOutputUsageError {
    #[error("the batch output is {observed} bytes; the maximum is {limit}")]
    JsonTooLarge { observed: usize, limit: usize },
    #[error("a batch output line is {observed} bytes; the maximum is {limit}")]
    LineTooLarge { observed: usize, limit: usize },
    #[error("the batch output has more than {limit} lines")]
    TooManyLines { limit: usize },
    #[error("the batch output totals overflowed at {field}")]
    Overflow { field: &'static str },
    #[error("the batch output usage totals could not be summed: {source}")]
    UsageSum {
        #[from]
        source: UsageSumError,
    },
}

// One output line that cannot be read or priced; the batch still bills its other lines.
#[derive(Debug, Error)]
pub enum BatchLineError {
    #[error("the {field} field must be an object")]
    NotAnObject { field: &'static str },
    #[error("the {field} field must be an integer")]
    NotAnInteger { field: &'static str },
    #[error("the {field} field must be a number")]
    NotANumber { field: &'static str },
    #[error("the provider usage object could not be read: {source}")]
    AnthropicUsage {
        #[from]
        source: AnthropicUsageError<Infallible>,
    },
    #[error("the provider usage object could not be converted: {source}")]
    LoggingUsage {
        #[from]
        source: LoggingUsageError,
    },
    #[error("the provider usage object could not be priced: {source}")]
    Pricing {
        #[from]
        source: UsageConversionError,
    },
    #[error("the responses usage object could not be read: {source}")]
    ResponsesUsage {
        #[from]
        source: ResponsesUsageError,
    },
    #[error("the batch cost could not be computed: {source}")]
    BatchCost {
        #[from]
        source: BatchCostError,
    },
    #[error("the prompt token detail could not be computed: {source}")]
    TokenCost {
        #[from]
        source: TokenCostError,
    },
    #[error("the OCR batch cost could not be computed: {source}")]
    OcrCost {
        #[from]
        source: OcrCostError,
    },
    #[error("the provider payload could not be read as JSON: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
    #[error("the converse usage object matches the converse shape but cannot be read")]
    ConverseUsage,
}

#[derive(Debug, Error)]
pub enum BatchEntryTokenError {
    #[error("the batch input entry must be an object")]
    NotAnEntry,
    #[error("the {field} field must be an object")]
    NotAnObject { field: &'static str },
    #[error("the {field} field must be a string")]
    NotAString { field: &'static str },
    #[error("the messages field could not be read as chat messages: {source}")]
    Messages {
        #[from]
        source: serde_json::Error,
    },
    #[error("the token count failed: {source}")]
    Count {
        #[from]
        source: CountError,
    },
    #[error("the token count overflowed at {field}")]
    Overflow { field: &'static str },
}
