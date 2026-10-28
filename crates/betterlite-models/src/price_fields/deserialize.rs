use super::{PriceFieldError, parse_decimal_lexeme};
use rust_decimal::Decimal;
use serde::{Deserialize as _, Deserializer, de::Error as _};
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

pub(super) struct ExactDecimal;

impl<'de> serde_with::DeserializeAs<'de, Decimal> for ExactDecimal {
    fn deserialize_as<Input>(deserializer: Input) -> Result<Decimal, Input::Error>
    where
        Input: Deserializer<'de>,
    {
        read_price(deserializer, "decimal")
    }
}
