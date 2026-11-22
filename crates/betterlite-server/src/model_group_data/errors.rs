use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_models::decisions::Json;
use betterlite_router::model_catalog_lookup::error::LookupError;

use crate::proxy_helpers::checks::{Detail, HttpException};

// _http_error: every caller-visible failure in this module answers with detail {"error": message}.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct HttpFailure {
    pub status: StatusCode,
    pub message: String,
}

impl From<HttpFailure> for HttpException {
    fn from(failure: HttpFailure) -> Self {
        Self {
            status: failure.status,
            detail: Some(Detail::Map(BTreeMap::from([(
                "error".to_owned(),
                Json::Text(failure.message),
            )]))),
            message: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GroupDataError {
    #[error("{0}")]
    Http(HttpFailure),
    #[error("{message}")]
    ValueError { message: String },
    #[error("{message}")]
    Runtime { message: String },
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Catalog(#[from] LookupError),
    #[error("a stored row is not valid JSON")]
    Row(#[source] serde_json::Error),
}
