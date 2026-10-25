pub mod filters;
pub mod metrics;
pub mod queries;
pub mod rollup;

pub const PTU_SENTINEL_API_KEY: &str = "__ptu_flat_cost__";
pub const USAGE_TOP_API_KEYS_MAX: i64 = 1000;
pub const USAGE_KEY_PAGE_MAX: i64 = 100;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DailyActivitySqlError {
    #[error("api_key_limit must be between 1 and 1000")]
    ApiKeyLimit,
    #[error("limit must be between 1 and 100")]
    PageLimit,
    #[error("offset must be non-negative")]
    Offset,
    #[error("timezone adjustment overflow")]
    Timezone,
}
mod tests;
