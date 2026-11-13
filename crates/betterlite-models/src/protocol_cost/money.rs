use rust_decimal::Decimal;

use super::error::ProtocolCostError;
use crate::decisions::Json;
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Strings {
    Float,
    Reject,
}

// Null is absent. A2A reads numeric strings the way Python's float() does; MCP takes numbers only.
pub(super) fn money(
    field: &'static str,
    value: &Json,
    strings: Strings,
) -> Result<Option<Decimal>, ProtocolCostError> {
    let invalid = ProtocolCostError::InvalidMoney { field };
    match value {
        Json::Null => Ok(None),
        Json::Number(number) => parse_decimal_lexeme(&number.to_string())
            .map(Some)
            .map_err(|_source| invalid),
        Json::Text(text) if strings == Strings::Float => {
            let trimmed = text.trim();
            let mut previous_is_digit = false;
            let mut characters = trimmed.chars().peekable();
            while let Some(character) = characters.next() {
                let next_is_digit = characters.peek().is_some_and(char::is_ascii_digit);
                if character == '_' && !(previous_is_digit && next_is_digit) {
                    return Err(invalid);
                }
                previous_is_digit = character.is_ascii_digit();
            }
            parse_decimal_lexeme(&trimmed.replace('_', ""))
                .map(Some)
                .map_err(|_source| invalid)
        }
        Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => Err(invalid),
    }
}
