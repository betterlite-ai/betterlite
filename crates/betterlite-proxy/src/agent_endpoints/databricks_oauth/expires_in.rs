use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::IgnoredAny;

const DEFAULT_TTL_SECONDS: i64 = 3600;
const TOKEN_EXPIRY_BUFFER_SECONDS: i64 = 60;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ExpiresIn {
    Flag(bool),
    // text comes before Number, since Decimal also reads numeric strings and int() does not
    Text(String),
    Number(Decimal),
    Other(IgnoredAny),
}

// Python's int() on text: surrounding whitespace, one sign, and single underscores between digits.
fn parse_text(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let negative = trimmed.starts_with('-');
    let digits = trimmed.strip_prefix(['-', '+']).unwrap_or(trimmed);
    let well_formed = !digits.starts_with('_')
        && !digits.ends_with('_')
        && !digits.contains("__")
        && digits.chars().any(|digit| digit.is_ascii_digit())
        && digits
            .chars()
            .all(|digit| digit.is_ascii_digit() || digit == '_');
    if !well_formed {
        return None;
    }
    let plain: String = digits.chars().filter(|digit| *digit != '_').collect();
    let sign = if negative { "-" } else { "" };
    // Only ASCII digits remain, so a parse failure is an overflow.
    Some(
        format!("{sign}{plain}")
            .parse::<i64>()
            .unwrap_or(if negative { i64::MIN } else { i64::MAX }),
    )
}

// Python's int() on a number truncates toward zero.
fn truncate(number: Decimal) -> i64 {
    let saturated = if number.is_sign_negative() {
        i64::MIN
    } else {
        i64::MAX
    };
    i64::try_from(number.trunc()).unwrap_or(saturated)
}

// A value int() cannot read falls back to the default lifetime.
fn seconds(expires_in: Option<&ExpiresIn>) -> i64 {
    let parsed = match expires_in {
        Some(ExpiresIn::Flag(flag)) => Some(i64::from(*flag)),
        Some(ExpiresIn::Number(number)) => Some(truncate(*number)),
        Some(ExpiresIn::Text(text)) => parse_text(text),
        Some(ExpiresIn::Other(_)) | None => None,
    };
    parsed.unwrap_or(DEFAULT_TTL_SECONDS)
}

// The cache lifetime ends 60 seconds before the token does and never goes below zero.
#[must_use]
pub fn cache_ttl(expires_in: Option<&ExpiresIn>) -> u32 {
    let lifetime = seconds(expires_in)
        .saturating_sub(TOKEN_EXPIRY_BUFFER_SECONDS)
        .max(0);
    u32::try_from(lifetime).unwrap_or(u32::MAX)
}
