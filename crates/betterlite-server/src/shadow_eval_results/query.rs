use std::collections::BTreeMap;

use thiserror::Error;

use crate::shadow_eval_models::ShadowEvalTargetType;

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
    #[error("the query parameter {parameter} is invalid")]
    Invalid { parameter: &'static str },
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
    let limit = match values.get("limit") {
        Some(raw_limit) => parse_limit(raw_limit)?,
        None => DEFAULT_SHADOW_LIMIT,
    };
    let target_type = values
        .get("target_type")
        .map(|raw_type| parse_target_type(raw_type))
        .transpose()?;
    Ok(ShadowQuery {
        target_type,
        target_id: values.get("target_id").cloned(),
        limit,
    })
}

fn parse_limit(raw: &str) -> Result<i64, ShadowQueryError> {
    let parsed = raw
        .trim()
        .parse::<i64>()
        .map_err(|_error| ShadowQueryError::Invalid { parameter: "limit" })?;
    if (MIN_SHADOW_LIMIT..=MAX_SHADOW_LIMIT).contains(&parsed) {
        return Ok(parsed);
    }
    Err(ShadowQueryError::Invalid { parameter: "limit" })
}

fn parse_target_type(raw: &str) -> Result<ShadowEvalTargetType, ShadowQueryError> {
    match raw {
        "key" => Ok(ShadowEvalTargetType::Key),
        "team" => Ok(ShadowEvalTargetType::Team),
        "user" => Ok(ShadowEvalTargetType::User),
        _other => Err(ShadowQueryError::Invalid {
            parameter: "target_type",
        }),
    }
}
