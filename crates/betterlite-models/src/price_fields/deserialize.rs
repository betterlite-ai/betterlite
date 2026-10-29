use super::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer};
use serde_json::Number;

#[derive(Deserialize)]
#[serde(try_from = "Number")]
pub(super) struct ExactDecimal(Decimal);

impl TryFrom<Number> for ExactDecimal {
    type Error = DecimalLexemeError;

    fn try_from(number: Number) -> Result<Self, Self::Error> {
        let lexeme = number.to_string();
        parse_decimal_lexeme(&lexeme).map(Self)
    }
}

impl From<ExactDecimal> for Decimal {
    fn from(value: ExactDecimal) -> Self {
        value.0
    }
}

pub(crate) fn optional_price<'de, Input>(
    deserializer: Input,
) -> Result<Option<Decimal>, Input::Error>
where
    Input: Deserializer<'de>,
{
    let value = ExactDecimal::deserialize(deserializer)?;
    Ok(Some(Decimal::from(value)))
}
