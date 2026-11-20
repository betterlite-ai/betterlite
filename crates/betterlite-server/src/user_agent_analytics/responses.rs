use rust_decimal::Decimal;
use serde::Serialize;

use super::rows::TagSummaryRow;

#[derive(Debug, Serialize)]
pub(super) struct DistinctTagResponse {
    pub(super) tag: String,
}

#[derive(Debug, Serialize)]
pub(super) struct DistinctTagsResponse {
    pub(super) results: Vec<DistinctTagResponse>,
}

#[derive(Debug, Serialize)]
pub(super) struct TagActiveUsersResponse {
    pub(super) tag: String,
    pub(super) active_users: i64,
    pub(super) date: String,
    pub(super) period_start: Option<String>,
    pub(super) period_end: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ActiveUsersAnalyticsResponse {
    pub(super) results: Vec<TagActiveUsersResponse>,
}

#[derive(Debug, Serialize)]
pub(super) struct TagSummaryResponse {
    pub(super) results: Vec<TagSummaryMetrics>,
}

#[derive(Debug, Serialize)]
pub(super) struct TagSummaryMetrics {
    pub(super) tag: String,
    pub(super) unique_users: i64,
    pub(super) total_requests: i64,
    pub(super) successful_requests: i64,
    pub(super) failed_requests: i64,
    pub(super) total_tokens: i64,
    #[serde(serialize_with = "rust_decimal::serde::float::serialize")]
    pub(super) total_spend: Decimal,
}

impl From<TagSummaryRow> for TagSummaryMetrics {
    fn from(row: TagSummaryRow) -> Self {
        Self {
            tag: row.tag,
            unique_users: row.unique_users,
            total_requests: row.total_requests.unwrap_or(0),
            successful_requests: row.successful_requests.unwrap_or(0),
            failed_requests: row.failed_requests.unwrap_or(0),
            total_tokens: row.total_tokens.unwrap_or(0),
            total_spend: row.total_spend.unwrap_or_default(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct PerUserAnalyticsResponse {
    pub(super) results: Vec<PerUserMetric>,
    pub(super) total_count: i64,
    pub(super) page: i64,
    pub(super) page_size: i64,
    pub(super) total_pages: i64,
}

#[derive(Debug, Serialize)]
pub(super) struct PerUserMetric {
    pub(super) user_id: String,
    pub(super) user_email: Option<String>,
    pub(super) user_agent: Option<String>,
    pub(super) successful_requests: i64,
    pub(super) failed_requests: i64,
    pub(super) total_requests: i64,
    pub(super) total_tokens: i64,
    #[serde(serialize_with = "rust_decimal::serde::float::serialize")]
    pub(super) spend: Decimal,
}
