use thiserror::Error;

use crate::tool_signals::error::ToolSignalError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum RateField {
    SearchContextSizeLow,
    SearchContextSizeMedium,
    SearchContextSizeHigh,
    FileSearchCostPerGbPerDay,
    FileSearchCostPer1kCalls,
    VectorStoreCostPerGbPerDay,
    #[strum(serialize = "computer_use_input_cost_per_1k_tokens")]
    ComputerInputPer1k,
    #[strum(serialize = "computer_use_output_cost_per_1k_tokens")]
    ComputerOutputPer1k,
    CodeInterpreterCostPerSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ToolFeeError {
    #[error("the selected {field} rate is null, so the fee cannot be computed")]
    NullRate { field: RateField },
    #[error("tool fee arithmetic left the decimal range")]
    Overflow,
    #[error(transparent)]
    Signal(#[from] ToolSignalError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("the price lookup failed")]
pub struct LookupFailed;
