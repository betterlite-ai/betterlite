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
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in form_urlencoded::parse(raw) {
        values.insert(name.into_owned(), value.into_owned());
    }
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
    let Ok(parsed) = raw.trim().parse::<i64>() else {
        return Err(unparsed_limit(raw.trim()));
    };
    if parsed < MIN_SHADOW_LIMIT {
        return Err(limit_error(
            "greater_than_equal",
            "Input should be greater than or equal to 1",
        ));
    }
    if parsed > MAX_SHADOW_LIMIT {
        return Err(limit_error(
            "less_than_equal",
            "Input should be less than or equal to 200",
        ));
    }
    Ok(parsed)
}

const fn limit_error(kind: &'static str, msg: &'static str) -> ShadowQueryError {
    ShadowQueryError::Invalid {
        parameter: "limit",
        kind,
        msg,
    }
}

// pydantic's int has no width, so a plain integer past i64 still faces the 1..200 bounds
fn unparsed_limit(text: &str) -> ShadowQueryError {
    let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
    if unsigned.is_empty() || !unsigned.bytes().all(|byte| byte.is_ascii_digit()) {
        return limit_error(
            "int_parsing",
            "Input should be a valid integer, unable to parse string as an integer",
        );
    }
    if text.starts_with('-') {
        return limit_error(
            "greater_than_equal",
            "Input should be greater than or equal to 1",
        );
    }
    limit_error(
        "less_than_equal",
        "Input should be less than or equal to 200",
    )
}
