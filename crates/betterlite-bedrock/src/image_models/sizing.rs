use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SizeError {
    #[error("not enough values to unpack (expected 2, got {0})")]
    NotEnough(usize),
    #[error("too many values to unpack (expected 2)")]
    TooMany,
    #[error("invalid literal for int() with base 10: '{0}'")]
    InvalidInt(String),
}

// Mirrors Python's int(str): trims whitespace, allows one leading sign and single underscores between digits
pub fn parse_python_int(text: &str) -> Result<i64, SizeError> {
    let trimmed = text.trim();
    let (sign, digits) = trimmed.strip_prefix('-').map_or_else(
        || ("", trimmed.strip_prefix('+').unwrap_or(trimmed)),
        |rest| ("-", rest),
    );
    if !has_valid_underscores(digits) {
        return Err(SizeError::InvalidInt(text.to_owned()));
    }
    let cleaned: String = digits
        .chars()
        .filter(|character| *character != '_')
        .collect();
    if cleaned.is_empty() || !cleaned.chars().all(|character| character.is_ascii_digit()) {
        return Err(SizeError::InvalidInt(text.to_owned()));
    }
    format!("{sign}{cleaned}")
        .parse::<i64>()
        .map_err(|_source| SizeError::InvalidInt(text.to_owned()))
}

fn has_valid_underscores(digits: &str) -> bool {
    if digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return false;
    }
    true
}

// Mirrors width, height = size.split("x") followed by int(width), int(height).
// size.split('x') always yields at least one item, so the first slot is never empty.
pub fn split_size(size: &str) -> Result<(i64, i64), SizeError> {
    let mut iter = size.split('x');
    let width = iter.next().unwrap_or_default();
    let maybe_height = iter.next();
    let extra = iter.next();
    match (maybe_height, extra) {
        (Some(height), None) => Ok((parse_python_int(width)?, parse_python_int(height)?)),
        (None, None) => Err(SizeError::NotEnough(1)),
        _ => Err(SizeError::TooMany),
    }
}
