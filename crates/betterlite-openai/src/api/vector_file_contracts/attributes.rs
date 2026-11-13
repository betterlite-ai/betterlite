use betterlite_models::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Number;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum AttributeWire {
    Text(String),
    Boolean(bool),
    Number(Number),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "AttributeWire")]
#[serde(untagged)]
pub enum AttributeValue {
    Text(String),
    Boolean(bool),
    Number(#[serde(with = "rust_decimal::serde::arbitrary_precision")] Decimal),
}

impl TryFrom<AttributeWire> for AttributeValue {
    type Error = DecimalLexemeError;

    fn try_from(wire: AttributeWire) -> Result<Self, Self::Error> {
        match wire {
            AttributeWire::Text(text) => Ok(Self::Text(text)),
            AttributeWire::Boolean(flag) => Ok(Self::Boolean(flag)),
            AttributeWire::Number(number) => {
                parse_decimal_lexeme(&number.to_string()).map(Self::Number)
            }
        }
    }
}
