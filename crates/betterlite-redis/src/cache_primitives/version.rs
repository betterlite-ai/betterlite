mod tests;

use std::borrow::Cow;

pub enum RedisVersionValue {
    Text(String),
    Float(f64),
    Integer(i64),
}

#[must_use]
pub fn parse_major_version(value: &RedisVersionValue, default_major_version: u32) -> u32 {
    let text: Cow<'_, str> = match value {
        RedisVersionValue::Text(text) => Cow::Borrowed(text.as_str()),
        RedisVersionValue::Float(float) => Cow::Owned(float.to_string()),
        RedisVersionValue::Integer(integer) => Cow::Owned(integer.to_string()),
    };
    let trimmed = text.trim();
    if trimmed == "Unknown" {
        return default_major_version;
    }
    let candidate = trimmed.split_once('.').map_or(trimmed, |(major, _)| major);
    candidate.parse().unwrap_or(default_major_version)
}
