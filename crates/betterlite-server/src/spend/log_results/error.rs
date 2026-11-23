use std::collections::BTreeMap;

use axum::response::{IntoResponse, Response};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::table_ops::rows::RowError;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

use super::dates::LegacyDateError;
use super::query::LegacyQueryError;
use crate::auth::auth_resource_grants::denial_response;
use crate::spend::log_payload::daily_summary::SummaryError;

const ERROR_PREFIX: &str = "/spend/logs Error";
const INTERNAL_ERROR_TYPE: &str = "internal_error";
// getattr(e, "param", "None") stores the text, so the error body carries the string "None"
const NONE_PARAM: &str = "None";
const NO_DATABASE_MESSAGE: &str = "Database not connected. Connect a database to your proxy - https://docs.litellm.ai/docs/simple_proxy#managing-auth---virtual-keys";

#[derive(Debug, thiserror::Error)]
pub enum LogResultsError {
    // FastAPI rejects an unparsable query parameter before the endpoint runs
    #[error(transparent)]
    Query(#[from] LegacyQueryError),
    #[error("the caller's identity could not be read")]
    Identity(#[source] serde_json::Error),
    #[error(transparent)]
    Dates(#[from] LegacyDateError),
    #[error("the database is not connected")]
    NoDatabase,
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Summary(#[from] SummaryError),
}

// the way a failing store or reader reports: Prisma answers a failed query, an HTTPException a status
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Row(#[from] RowError),
    // boxed to keep the error under result_large_err's 128-byte limit
    #[error(transparent)]
    Proxy(#[from] Box<ProxyException>),
    #[error("an HTTP exception was raised while reading spend logs")]
    Http { status: u16, detail: String },
}

fn spend_logs_error(message: &str) -> Response {
    let exception = ProxyException::new(ProxyExceptionInit {
        message: format!("{ERROR_PREFIX}{message}"),
        error_type: String::from(INTERNAL_ERROR_TYPE),
        param: Some(String::from(NONE_PARAM)),
        code: ExceptionCode::Status(500),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    });
    denial_response(&exception)
}

// HTTPException: its detail becomes the message and its status the code
fn http_exception_response(status: u16, detail: &str) -> Response {
    denial_response(&ProxyException::new(ProxyExceptionInit {
        message: detail.to_owned(),
        error_type: String::from(INTERNAL_ERROR_TYPE),
        param: Some(String::from(NONE_PARAM)),
        code: ExceptionCode::Status(i64::from(status)),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    }))
}

fn store_response(error: &StoreError) -> Response {
    match error {
        StoreError::Proxy(exception) => denial_response(exception),
        StoreError::Http { status, detail } => http_exception_response(*status, detail),
        StoreError::Database(source) => spend_logs_error(&source.to_string()),
        StoreError::Row(source) => spend_logs_error(&source.to_string()),
    }
}

impl IntoResponse for LogResultsError {
    fn into_response(self) -> Response {
        match self {
            Self::Query(error) => error.into_response(),
            Self::Identity(source) => spend_logs_error(&source.to_string()),
            Self::Dates(error) => spend_logs_error(&error.to_string()),
            Self::NoDatabase => spend_logs_error(NO_DATABASE_MESSAGE),
            Self::Store(error) => store_response(&error),
            Self::Summary(error) => spend_logs_error(&error.to_string()),
        }
    }
}
