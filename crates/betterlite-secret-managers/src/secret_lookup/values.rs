use secrecy::{ExposeSecret as _, SecretString};

use super::SecretValue;
use super::text_to_bool;

pub(super) fn environment_value(text: &str) -> SecretValue {
    text_to_bool(text).map_or_else(
        || SecretValue::Text(SecretString::from(text)),
        SecretValue::Bool,
    )
}

pub(super) fn literal_value(text: SecretString) -> SecretValue {
    let trimmed = text
        .expose_secret()
        .trim_start_matches([' ', '\t'])
        .trim_end();
    if trimmed == "True" {
        return SecretValue::Bool(true);
    }
    if trimmed == "False" {
        return SecretValue::Bool(false);
    }
    SecretValue::Text(text)
}
