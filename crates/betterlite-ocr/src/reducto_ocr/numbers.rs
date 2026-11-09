use std::cmp::Ordering;

use crate::json::Json;

// LiteLLM accepts whole numbers, numeric strings and booleans for Reducto usage counts
pub(super) fn lax_i64(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().and_then(integral_i64)),
        Json::Text(text) => integer_string(text.trim()),
        Json::Bool(flag) => Some(i64::from(*flag)),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

// LiteLLM's LaxI64 accepts an integral float only below 2^63; the cast there does not saturate
fn integral_i64(value: f64) -> Option<i64> {
    if value.fract() != 0.0_f64 || value >= 2.0_f64.powi(63_i32) {
        return None;
    }
    truncated_i64(value)
}

// the page-number reference saturates a f64 holding exactly 2^63 at the top of the i64 range
pub(super) fn truncated_i64(value: f64) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    let truncated = value.trunc();
    if truncated.total_cmp(&2.0_f64.powi(63_i32)) == Ordering::Equal {
        return Some(i64::MAX);
    }
    format!("{truncated:.0}").parse().ok()
}

fn integer_string(value: &str) -> Option<i64> {
    let integer = match value.split_once('.') {
        Some((integer, fraction)) => {
            if fraction.is_empty() || !fraction.bytes().all(|byte| byte == b'0') {
                return None;
            }
            integer
        }
        None => value,
    };
    if integer.starts_with('_') || integer.ends_with('_') || integer.contains("__") {
        return None;
    }
    let digits = integer.strip_prefix(['+', '-']).unwrap_or(integer);
    if digits.is_empty()
        || digits.starts_with('_')
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_')
    {
        return None;
    }
    integer.replace('_', "").parse().ok()
}
