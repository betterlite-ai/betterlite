use crate::price_fields::decimal_lexeme::DecimalLexemeError;

#[derive(Debug, thiserror::Error)]
pub enum SavingsEvidenceError {
    #[error("tokens saved total overflows a 64 bit counter")]
    SavedTokensOverflow,
    #[error("a token count is not a representable decimal")]
    TokenCount {
        #[source]
        source: DecimalLexemeError,
    },
    #[error("{field} must not be negative")]
    NegativeAmount { field: &'static str },
    #[error("amount must be a JSON number")]
    NonNumericAmount,
    #[error("amount is out of the representable decimal range")]
    AmountOutOfRange {
        #[source]
        source: DecimalLexemeError,
    },
}
