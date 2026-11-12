use rust_decimal::Decimal;

use crate::tool_fees::error::{RateField, ToolFeeError};

// A model-info key keeps whether it is missing or present with null, which LiteLLM treats differently.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Rate {
    #[default]
    Absent,
    Null,
    Value(Decimal),
}

impl Rate {
    pub const fn or_zero(self, field: RateField) -> Result<Decimal, ToolFeeError> {
        match self {
            Self::Absent => Ok(Decimal::ZERO),
            Self::Null => Err(ToolFeeError::NullRate { field }),
            Self::Value(value) => Ok(value),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchContextRates {
    pub low: Rate,
    pub medium: Rate,
    pub high: Rate,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolModelInfo {
    pub litellm_provider: Option<String>,
    pub search_context: SearchContextRates,
    pub file_search_per_gb_per_day: Rate,
    pub file_search_per_1k_calls: Rate,
    pub vector_store_per_gb_per_day: Rate,
    pub computer_input_per_1k: Rate,
    pub computer_output_per_1k: Rate,
    pub code_interpreter_per_session: Rate,
}
