use betterlite_models::decisions::Json;

// Python's int() truncates toward zero; betterlite reads the decimal text exactly instead of a double
const MAX_DURATION_DIGITS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DurationValue {
    Auto,
    // the value truncated toward zero, written with the digits the input gave
    Numeric(String),
}

enum Exponent {
    Value(i64),
    TooLarge,
    TooSmall,
}

#[must_use]
pub fn duration_value(value: &Json) -> Option<DurationValue> {
    match value {
        Json::Text(text) if text == "auto" => Some(DurationValue::Auto),
        Json::Text(text) => truncate_decimal(text).map(DurationValue::Numeric),
        Json::Number(number) => truncate_decimal(&number.to_string()).map(DurationValue::Numeric),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

#[must_use]
pub fn numeric_duration(value: Option<&Json>) -> Option<String> {
    match duration_value(value?)? {
        DurationValue::Auto => None,
        DurationValue::Numeric(text) => Some(text),
    }
}

fn truncate_decimal(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let (negative, unsigned) = sign_parts(trimmed);
    let (mantissa, exponent) = split_exponent(unsigned)?;
    let (digits, fraction_len) = mantissa_digits(mantissa)?;
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return Some("0".to_owned());
    }
    let shift = match exponent {
        Exponent::Value(value) => value.saturating_sub(fraction_len),
        Exponent::TooLarge => return None,
        Exponent::TooSmall => return Some("0".to_owned()),
    };
    let truncated = truncated_digits(significant.to_owned(), shift)?;
    Some(signed_integer_text(negative, &truncated))
}

fn split_exponent(text: &str) -> Option<(&str, Exponent)> {
    match text.split_once(['e', 'E']) {
        None => Some((text, Exponent::Value(0))),
        Some((mantissa, exponent)) => {
            let (negative, digits) = sign_parts(exponent);
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let Ok(magnitude) = digits.parse::<i64>() else {
                return Some((
                    mantissa,
                    if negative {
                        Exponent::TooSmall
                    } else {
                        Exponent::TooLarge
                    },
                ));
            };
            let value = if negative {
                magnitude.checked_neg()?
            } else {
                magnitude
            };
            Some((mantissa, Exponent::Value(value)))
        }
    }
}

fn sign_parts(text: &str) -> (bool, &str) {
    let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
    (text.starts_with('-'), unsigned)
}

fn mantissa_digits(mantissa: &str) -> Option<(String, i64)> {
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let fraction_len = i64::try_from(fraction.len()).ok()?;
    let mut digits = String::with_capacity(whole.len().saturating_add(fraction.len()));
    digits.push_str(whole);
    digits.push_str(fraction);
    Some((digits, fraction_len))
}

fn truncated_digits(digits: String, shift: i64) -> Option<String> {
    if shift >= 0 {
        let zeros = usize::try_from(shift).ok()?;
        if digits.len().checked_add(zeros)? > MAX_DURATION_DIGITS {
            return None;
        }
        let mut padded = digits;
        padded.push_str(&"0".repeat(zeros));
        return Some(padded);
    }
    let dropped = usize::try_from(shift.unsigned_abs()).ok()?;
    if dropped >= digits.len() {
        return Some(String::new());
    }
    let kept = digits.len().checked_sub(dropped)?;
    let mut truncated = digits;
    truncated.truncate(kept);
    Some(truncated)
}

fn signed_integer_text(negative: bool, digits: &str) -> String {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        return "0".to_owned();
    }
    if negative {
        format!("-{trimmed}")
    } else {
        trimmed.to_owned()
    }
}
