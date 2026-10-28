use super::{PriceFieldError, parse_decimal_lexeme};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, de::Error as _};
use serde_json::Number;

pub(super) fn read_price<'de, Input>(
    deserializer: Input,
    path: &str,
) -> Result<Decimal, Input::Error>
where
    Input: Deserializer<'de>,
{
    let number = Option::<Number>::deserialize(deserializer)
        .map_err(|_source| {
            Input::Error::custom(PriceFieldError::NotNumber {
                path: path.to_owned(),
            })
        })?
        .ok_or_else(|| {
            Input::Error::custom(PriceFieldError::Null {
                path: path.to_owned(),
            })
        })?;
    let lexeme = number.to_string();
    parse_decimal_lexeme(&lexeme).map_err(|source| {
        Input::Error::custom(PriceFieldError::InvalidDecimal {
            path: path.to_owned(),
            value: lexeme,
            source,
        })
    })
}

#[derive(Deserialize)]
#[serde(try_from = "Number")]
pub(super) struct ExactDecimal(Decimal);

impl TryFrom<Number> for ExactDecimal {
    type Error = rust_decimal::Error;

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
