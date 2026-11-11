use betterlite_tokenizer::count::error::CountError;
use thiserror::Error;

use crate::cost::realtime::error::RealtimeCostError;
use crate::cost::usage::BillingUsageError;

#[derive(Debug, Error)]
pub enum PrepareError {
    #[error("the billing usage could not be read: {0}")]
    Usage(#[from] BillingUsageError),
    #[error("model is not set and the response does not name one")]
    ModelMissing,
    #[error("the {field} field is not a whole number")]
    Count { field: &'static str },
    #[error("the hidden parameter {field} must be a string or null")]
    Hidden { field: &'static str },
    #[error("the websocket service tiers could not be split: {0}")]
    Realtime(#[from] RealtimeCostError),
    #[error("token counting failed: {0}")]
    Counting(#[from] CountError),
}
