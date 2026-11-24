use axum::extract::Query;
use axum::http::Uri;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const HOURS_LOC: [&str; 2] = ["query", "hours"];

#[derive(Debug, Deserialize)]
struct HoursQuery {
    hours: Option<String>,
}

#[derive(Debug, Error)]
pub(super) enum HoursFailure {
    #[error("the hours query parameter is required")]
    Missing,
    #[error("the hours query parameter is not an integer")]
    NotInteger,
}

#[derive(Serialize)]
pub(super) struct ValidationBody {
    pub(super) detail: [ValidationIssue; 1],
}

#[derive(Serialize)]
pub(super) struct ValidationIssue {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) loc: [&'static str; 2],
    pub(super) msg: &'static str,
}

impl HoursFailure {
    #[must_use]
    pub(super) const fn issue(&self) -> ValidationIssue {
        match self {
            Self::Missing => ValidationIssue {
                kind: "missing",
                loc: HOURS_LOC,
                msg: "Field required",
            },
            Self::NotInteger => ValidationIssue {
                kind: "int_parsing",
                loc: HOURS_LOC,
                msg: "Input should be a valid integer, unable to parse string as an integer",
            },
        }
    }
}

// FastAPI validates the required `hours: int` query parameter before the handler, so this 422 comes before the role check
pub(super) fn parse_hours(uri: &Uri) -> Result<i64, HoursFailure> {
    let Ok(Query(query)) = Query::<HoursQuery>::try_from_uri(uri) else {
        return Err(HoursFailure::NotInteger);
    };
    let Some(text) = query.hours else {
        return Err(HoursFailure::Missing);
    };
    text.parse::<i64>()
        .map_err(|_source| HoursFailure::NotInteger)
}
