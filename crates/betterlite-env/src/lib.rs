use std::str::FromStr;

use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum EnvError {
    #[error("environment variable {name} must be a bool")]
    Bool { name: String },
    #[error("environment variable {name} must be a number")]
    Number { name: String },
}

pub fn flag(name: &str, text: &str) -> Result<Option<bool>, EnvError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.eq_ignore_ascii_case("true")
        || trimmed.eq_ignore_ascii_case("1")
        || trimmed.eq_ignore_ascii_case("yes")
        || trimmed.eq_ignore_ascii_case("on")
    {
        return Ok(Some(true));
    }
    if trimmed.eq_ignore_ascii_case("false")
        || trimmed.eq_ignore_ascii_case("0")
        || trimmed.eq_ignore_ascii_case("no")
        || trimmed.eq_ignore_ascii_case("off")
    {
        return Ok(Some(false));
    }
    Err(EnvError::Bool {
        name: name.to_owned(),
    })
}

pub fn number<T>(name: &str, text: &str) -> Result<Option<T>, EnvError>
where
    T: FromStr,
{
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if !trimmed
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.' | b'e' | b'E'))
    {
        return Err(EnvError::Number {
            name: name.to_owned(),
        });
    }
    // A generic parser's error may disclose the environment value.
    trimmed
        .parse::<T>()
        .map(Some)
        .map_err(|_source| EnvError::Number {
            name: name.to_owned(),
        })
}

mod tests;
