use betterlite_models::price_fields::decimal_lexeme::DecimalLexemeError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FileSearchToolsError {
    #[error("{field} must be {expected}")]
    WrongType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the response_cost hidden parameter is not a decimal number: {source}")]
    ResponseCost {
        #[source]
        source: DecimalLexemeError,
    },
    #[error("adding the response_cost hidden parameters overflows a decimal")]
    CostOverflow,
    #[error("the clock value is not a whole second in the signed 64-bit range")]
    ClockOutOfRange,
    #[error("the value could not be read as its wire type: {source}")]
    Wire {
        #[source]
        source: serde_json::Error,
    },
}
