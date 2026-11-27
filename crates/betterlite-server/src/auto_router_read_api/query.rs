use std::collections::BTreeMap;

use thiserror::Error;

// the byte bound the reads apply before decoding, so an oversized query never reaches the parser
pub const MAX_READ_QUERY_BYTES: usize = 0x0010_0000;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BenchmarksQuery {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub api_key: Option<String>,
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
    let user_id = values.get("user_id").cloned();
    if user_id.as_deref() == Some("") {
        return Err(ReadQueryError::Invalid {
            parameter: "user_id",
            kind: "string_too_short",
            msg: "String should have at least 1 character",
        });
    }
    Ok(BenchmarksQuery {
        start_date: values.get("start_date").cloned(),
        end_date: values.get("end_date").cloned(),
        api_key: values.get("api_key").cloned(),
        user_id,
    })
}

// session_id is required with no length bound, so an empty one still reaches the lookup
pub fn decode_session_query(raw: &[u8]) -> Result<SessionQuery, ReadQueryError> {
    let values = decode_pairs(raw)?;
    let Some(session_id) = values.get("session_id") else {
        return Err(ReadQueryError::Invalid {
            parameter: "session_id",
            kind: "missing",
            msg: "Field required",
        });
    };
    Ok(SessionQuery {
        session_id: session_id.clone(),
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
