use betterlite_models::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Number;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Number")]
pub struct Score(#[serde(with = "rust_decimal::serde::arbitrary_precision")] pub Decimal);

impl TryFrom<Number> for Score {
    type Error = DecimalLexemeError;

    fn try_from(number: Number) -> Result<Self, Self::Error> {
        let lexeme = number.to_string();
        parse_decimal_lexeme(&lexeme).map(Self)
    }
}
