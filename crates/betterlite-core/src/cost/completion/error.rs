use std::convert::Infallible;
use std::error::Error;
use std::num::TryFromIntError;

use betterlite_cost::adjustments::AdjustmentError;
use betterlite_models::tool_fees::error::ToolFeeError;
use thiserror::Error;

use super::breakdown::BreakdownError;
use super::usage::UsageConversionError;
use crate::cost::call_price::error::CallPriceError;
use crate::cost::context::error::BillingContextError;
use crate::cost::prepare::error::PrepareError;
use crate::cost::special::error::SpecialChargeError;

// A value one charge reads that LiteLLM's runtime would accept loosely; each is a typed error naming what was wrong.
#[derive(Debug, Error)]
pub enum ChargeInputError {
    #[error("the {field} count does not fit a signed 64-bit counter: {source}")]
    CharacterCount {
        field: &'static str,
        #[source]
        source: TryFromIntError,
    },
    #[error("an OCR price is not representable as a JSON number: {0}")]
    OcrRate(#[source] serde_json::Error),
    #[error("the hidden parameter {field} must be a string or null")]
    Hidden { field: &'static str },
    #[error("the response cost header is not a decimal number: {0}")]
    CostHeader(String),
    #[error("the charge is too large to represent")]
    Overflow,
    #[error("the clock reads a time before the Unix epoch or past the supported range")]
    Clock,
}

#[derive(Debug, Error)]
pub enum CompletionCostError<ImageError = Infallible>
where
    ImageError: Error + 'static,
{
    #[error(transparent)]
    Prepare(#[from] PrepareError),
    #[error(transparent)]
    Special(#[from] SpecialChargeError<ImageError>),
    #[error(transparent)]
    Usage(#[from] UsageConversionError),
    #[error(transparent)]
    Context(#[from] BillingContextError),
    #[error(transparent)]
    Price(#[from] CallPriceError),
    #[error(transparent)]
    Tools(#[from] ToolFeeError),
    #[error(transparent)]
    Adjustment(#[from] AdjustmentError),
    #[error(transparent)]
    Breakdown(#[from] BreakdownError),
    #[error(transparent)]
    Input(#[from] ChargeInputError),
    #[error("the websocket partition split produced partitions again")]
    NestedPartitions,
    #[error("no candidate model name was prepared")]
    NoCandidates,
}
