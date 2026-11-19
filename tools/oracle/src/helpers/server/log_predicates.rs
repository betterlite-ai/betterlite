use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::spend::log_query::sql::{self, SpendLogSearchCondition};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde::Deserialize;

const STAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S";
const DATE_FORMAT: &str = "%Y-%m-%d";

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Search {
        search: String,
        start: String,
        end: String,
        next_param_index: usize,
    },
    Status {
        #[serde(default)]
        status_filter: Option<String>,
    },
    Span {
        #[serde(default)]
        span_type: Option<String>,
    },
}

fn failure(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn parse_stamp(text: &str) -> Result<DateTime<Utc>, Box<HelperError>> {
    let trimmed = text.trim();
    let naive = NaiveDateTime::parse_from_str(trimmed, STAMP_FORMAT)
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(trimmed, DATE_FORMAT)
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
        });
    let Some(naive) = naive else {
        return Err(failure(
            "the case needs a '%Y-%m-%d %H:%M:%S' or '%Y-%m-%d' stamp",
        ));
    };
    Ok(DateTime::from_naive_utc_and_offset(naive, Utc))
}

fn search_answer(search: &str, start: &str, end: &str, next_param_index: usize) -> HelperResult {
    let start_inclusive = parse_stamp(start)?;
    let end_inclusive = parse_stamp(end)?;
    let condition =
        sql::spend_log_search_condition(search, start_inclusive, end_inclusive, next_param_index);
    let SpendLogSearchCondition {
        sql: statement,
        params,
    } = condition;
    let (search_value, window_start, window_end) = params;
    Ok(serde_json::json!({
        "sql": statement,
        "params": [
            search_value,
            window_start.to_rfc3339(),
            window_end.to_rfc3339(),
        ],
    }))
}

fn status_answer(status_filter: Option<&str>) -> HelperResult {
    match sql::status_filter_condition(status_filter) {
        Some(condition) => Ok(serde_json::to_value(condition)?),
        None => Ok(serde_json::json!({})),
    }
}

fn span_answer(span_type: Option<&str>) -> HelperResult {
    Ok(serde_json::to_value(sql::span_type_sql_condition(
        span_type,
    ))?)
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Search {
                search,
                start,
                end,
                next_param_index,
            } => search_answer(search, start, end, *next_param_index),
            Input::Status { status_filter } => status_answer(status_filter.as_deref()),
            Input::Span { span_type } => span_answer(span_type.as_deref()),
        }
    }
}
