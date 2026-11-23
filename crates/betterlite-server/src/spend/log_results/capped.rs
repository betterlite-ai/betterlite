use axum::Json;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use betterlite_db::table_ops::rows::SpendLogRow;

use super::error::StoreError;
use super::rows::SpendLogRowsQuery;
use super::store::LogResultsStore;
use crate::spend::log_query::sql::SPEND_LOGS_COUNT_CAP;

pub const TRUNCATION_HEADER: &str = "x-litellm-spend-logs-truncated";
const TRUNCATION_VALUE: &str = "true";

pub struct CappedRows {
    pub rows: Vec<SpendLogRow>,
    pub truncated: bool,
}

// _find_spend_logs: a full page may hide more rows, so the response carries the truncation header
pub async fn find_spend_logs(
    store: &dyn LogResultsStore,
    query: &SpendLogRowsQuery,
) -> Result<CappedRows, StoreError> {
    let rows = store.spend_logs(query).await?;
    let truncated = usize::try_from(SPEND_LOGS_COUNT_CAP).is_ok_and(|cap| cap == rows.len());
    if truncated {
        tracing::warn!(
            "/spend/logs result truncated to the {SPEND_LOGS_COUNT_CAP} most recent rows; use /spend/logs/v2 for paginated access"
        );
    }
    Ok(CappedRows { rows, truncated })
}

impl IntoResponse for CappedRows {
    fn into_response(self) -> Response {
        let mut headers = HeaderMap::new();
        if self.truncated {
            headers.insert(
                TRUNCATION_HEADER,
                HeaderValue::from_static(TRUNCATION_VALUE),
            );
        }
        (headers, Json(self.rows)).into_response()
    }
}
