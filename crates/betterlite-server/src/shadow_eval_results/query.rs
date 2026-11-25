use std::collections::BTreeMap;

use thiserror::Error;

use crate::shadow_eval_models::ShadowEvalTargetType;
use crate::shadow_result_aggregation::targets::parse_target_type;

// the byte bound the read applies before decoding, so an oversized query never reaches the parser
pub const MAX_SHADOW_QUERY_BYTES: usize = 0x0010_0000;
pub const DEFAULT_SHADOW_LIMIT: i64 = 50;
pub const MIN_SHADOW_LIMIT: i64 = 1;
pub const MAX_SHADOW_LIMIT: i64 = 200;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShadowQuery {
    pub target_type: Option<ShadowEvalTargetType>,
    pub target_id: Option<String>,
    pub limit: i64,
}

// FastAPI answers 422 for a parameter its Query declaration rejects, before the endpoint runs
#[derive(Debug, Eq, Error, PartialEq)]
pub enum ShadowQueryError {
    #[error("the shadow eval query has {bytes} bytes, past the {limit}-byte limit")]
    TooLarge { bytes: usize, limit: usize },
    // the pydantic error type and message the validation body carries for the parameter
    #[error("the query parameter {parameter} is invalid")]
    Invalid {
        parameter: &'static str,
        kind: &'static str,
        msg: &'static str,
    },
}

// Bound first, decode second; a repeated parameter keeps its last value, as Starlette reads a scalar
pub fn decode_shadow_query(raw: &[u8]) -> Result<ShadowQuery, ShadowQueryError> {
    if raw.len() > MAX_SHADOW_QUERY_BYTES {
        return Err(ShadowQueryError::TooLarge {
            bytes: raw.len(),
            limit: MAX_SHADOW_QUERY_BYTES,
        });
    }
    let values: BTreeMap<String, String> = form_urlencoded::parse(raw).into_owned().collect();
    // FastAPI validates in the endpoint's parameter order, so target_type fails before limit
    let target_type = values
        .get("target_type")
        .map(|raw_type| {
            parse_target_type(raw_type).map_err(|_error| ShadowQueryError::Invalid {
                parameter: "target_type",
                kind: "literal_error",
                msg: "Input should be 'key', 'team' or 'user'",
            })
        })
        .transpose()?;
    let limit = match values.get("limit") {
        Some(raw_limit) => parse_limit(raw_limit)?,
        None => DEFAULT_SHADOW_LIMIT,
    };
    Ok(ShadowQuery {
        target_type,
        target_id: values.get("target_id").cloned(),
        limit,
    })
}

fn parse_limit(raw: &str) -> Result<i64, ShadowQueryError> {
    let limit = match json_int(raw) {
        Ok(limit) => limit,
        Err(JsonInt::TooLong) => return Err(limit_error("int_parsing_size", SIZE)),
        Err(JsonInt::Invalid) => clean_int(raw)
            .as_deref()
            .and_then(|cleaned| json_int(cleaned).ok())
            .ok_or_else(|| limit_error("int_parsing", PARSING))?,
    };
    if limit < MIN_SHADOW_LIMIT {
        return Err(limit_error("greater_than_equal", AT_LEAST_ONE));
    }
    if limit > MAX_SHADOW_LIMIT {
        return Err(limit_error("less_than_equal", AT_MOST_TWO_HUNDRED));
    }
    Ok(limit)
}

// the messages pydantic's int reader reports for a string it cannot read, and for its two bounds
const PARSING: &str = "Input should be a valid integer, unable to parse string as an integer";
const SIZE: &str = "Unable to parse input string as an integer, exceeded maximum size";
const AT_LEAST_ONE: &str = "Input should be greater than or equal to 1";
const AT_MOST_TWO_HUNDRED: &str = "Input should be less than or equal to 200";

const fn limit_error(kind: &'static str, msg: &'static str) -> ShadowQueryError {
    ShadowQueryError::Invalid {
        parameter: "limit",
        kind,
        msg,
    }
}

// pydantic's plain JSON-integer read: a sign, no leading zero, digits only, under 4300 bytes with the sign
fn json_int(text: &str) -> Result<i64, JsonInt> {
    let negative = text.starts_with('-');
    let body = text.strip_prefix('-').unwrap_or(text);
    let digits = body.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || (body.starts_with('0') && digits > 1) {
        return Err(JsonInt::Invalid);
    }
    if digits > (if negative { 4_299_usize } else { 4_300_usize }) {
        return Err(JsonInt::TooLong);
    }
    if digits != body.len() {
        return Err(JsonInt::Invalid);
    }
    let magnitude = body.parse::<i64>().unwrap_or(i64::MAX);
    let negated = 0_i64.saturating_sub(magnitude);
    Ok(if negative { negated } else { magnitude })
}

// how pydantic's plain read fails: past its byte cap, or not a JSON integer at all
enum JsonInt {
    TooLong,
    Invalid,
}

// the retry pydantic makes after that read fails: trim, one sign, leading zeros, a .0 tail, underscores
fn clean_int(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let (sign, magnitude) = match trimmed.strip_prefix(['-', '+']) {
        Some(rest) if rest.starts_with(['-', '+']) => return None,
        Some(rest) if trimmed.starts_with('-') => ("-", rest),
        Some(rest) => ("", rest),
        None => ("", trimmed),
    };
    let zeros = strip_leading_zeros(magnitude)?;
    let digits = match zeros.split_once('.') {
        Some((whole, fraction))
            if !fraction.is_empty() && fraction.bytes().all(|byte| byte == b'0') =>
        {
            whole
        }
        Some(_) | None => zeros,
    };
    let separators = !digits.starts_with('_') && !digits.ends_with('_') && !digits.contains("__");
    if digits.contains('_') && separators {
        return Some(format!("{sign}{}", digits.replace('_', "")));
    }
    Some(format!("{sign}{digits}"))
}

// pydantic keeps one zero, walks back to the zero before a dot, and drops underscores among zeros
fn strip_leading_zeros(text: &str) -> Option<&str> {
    if text.starts_with(|character: char| matches!(character, '1'..='9' | '-')) {
        return Some(text);
    }
    text.strip_prefix('0')?;
    for (index, character) in text.char_indices() {
        match character {
            '0' | '_' => {}
            '1'..='9' | '-' => return text.get(index..),
            '.' => return text.get(index.saturating_sub(1)..),
            _ => return None,
        }
    }
    text.get(text.len().saturating_sub(1)..)
}
