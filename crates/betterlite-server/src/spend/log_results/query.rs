use std::collections::BTreeMap;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

pub const API_KEY_PARAM: &str = "api_key";
pub const END_DATE_PARAM: &str = "end_date";
pub const REQUEST_ID_PARAM: &str = "request_id";
pub const START_DATE_PARAM: &str = "start_date";
pub const SUMMARIZE_PARAM: &str = "summarize";
pub const USER_ID_PARAM: &str = "user_id";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LegacySpendQuery {
    pub api_key: Option<String>,
    pub end_date: Option<String>,
    pub request_id: Option<String>,
    pub start_date: Option<String>,
    pub summarize: bool,
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the summarize query parameter is not a boolean")]
pub struct LegacyQueryError;

#[derive(Debug, Clone, Serialize)]
pub struct QueryIssue {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub loc: [&'static str; 2],
    pub msg: &'static str,
}

#[derive(Serialize)]
struct DetailBody {
    detail: [QueryIssue; 1],
}

// the strings pydantic accepts as a boolean for a query parameter
fn boolean(text: &str) -> Option<bool> {
    match text.to_ascii_lowercase().as_str() {
        "0" | "off" | "f" | "false" | "n" | "no" => Some(false),
        "1" | "on" | "t" | "true" | "y" | "yes" => Some(true),
        _ => None,
    }
}

// the legacy listing's query parameters: summarize is the only typed one and defaults to true
pub fn decode_legacy_spend_query(raw: &[u8]) -> Result<LegacySpendQuery, LegacyQueryError> {
    let values: BTreeMap<String, String> = form_urlencoded::parse(raw).into_owned().collect();
    let summarize = values
        .get(SUMMARIZE_PARAM)
        .map_or(Ok(true), |text| boolean(text).ok_or(LegacyQueryError))?;
    Ok(LegacySpendQuery {
        api_key: values.get(API_KEY_PARAM).cloned(),
        end_date: values.get(END_DATE_PARAM).cloned(),
        request_id: values.get(REQUEST_ID_PARAM).cloned(),
        start_date: values.get(START_DATE_PARAM).cloned(),
        summarize,
        user_id: values.get(USER_ID_PARAM).cloned(),
    })
}

impl IntoResponse for LegacyQueryError {
    fn into_response(self) -> Response {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(DetailBody {
                detail: [QueryIssue {
                    kind: "bool_parsing",
                    loc: ["query", SUMMARIZE_PARAM],
                    msg: "Input should be a valid boolean, unable to interpret input",
                }],
            }),
        )
            .into_response()
    }
}
