use std::collections::BTreeMap;

use thiserror::Error;

// the byte bound the reads apply before decoding, so an oversized query never reaches the parser
pub const MAX_READ_QUERY_BYTES: usize = 0x0010_0000;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BenchmarksQuery {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub key_hash: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionQuery {
    pub session_id: String,
}

// FastAPI answers 422 for a parameter its Query declaration rejects, before the endpoint runs
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ReadQueryError {
    #[error("the auto-router read query has {bytes} bytes, past the {limit}-byte limit")]
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
pub fn decode_benchmarks_query(raw: &[u8]) -> Result<BenchmarksQuery, ReadQueryError> {
    let values = decode_pairs(raw)?;
    Ok(BenchmarksQuery {
        start_date: values.get("start_date").cloned(),
        end_date: values.get("end_date").cloned(),
        key_hash: values.get("api_key").cloned(),
        user_id: at_least_one_character(&values, "user_id")?,
    })
}

// session_id is required with no length bound, so an empty one still reaches the lookup
pub fn decode_session_query(raw: &[u8]) -> Result<SessionQuery, ReadQueryError> {
    let values = decode_pairs(raw)?;
    Ok(SessionQuery {
        session_id: required(&values, "session_id")?,
    })
}

// the pydantic min_length=1 rule on the optional user filter
fn at_least_one_character(
    values: &BTreeMap<String, String>,
    parameter: &'static str,
) -> Result<Option<String>, ReadQueryError> {
    let value = values.get(parameter).cloned();
    if value.as_deref() == Some("") {
        return Err(ReadQueryError::Invalid {
            parameter,
            kind: "string_too_short",
            msg: "String should have at least 1 character",
        });
    }
    Ok(value)
}

// the required parameter, with the pydantic error FastAPI answers for a missing field
fn required(
    values: &BTreeMap<String, String>,
    parameter: &'static str,
) -> Result<String, ReadQueryError> {
    values
        .get(parameter)
        .cloned()
        .ok_or(ReadQueryError::Invalid {
            parameter,
            kind: "missing",
            msg: "Field required",
        })
}

fn decode_pairs(raw: &[u8]) -> Result<BTreeMap<String, String>, ReadQueryError> {
    if raw.len() > MAX_READ_QUERY_BYTES {
        return Err(ReadQueryError::TooLarge {
            bytes: raw.len(),
            limit: MAX_READ_QUERY_BYTES,
        });
    }
    Ok(form_urlencoded::parse(raw).into_owned().collect())
}
