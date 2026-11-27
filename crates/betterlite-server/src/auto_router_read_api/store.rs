use async_trait::async_trait;
use betterlite_db::managed_resource_rows::autorouter::AutoRouterSession;

use crate::benchmark_aggregates::row::SessionAggRow;
use crate::benchmark_aggregates::window::BenchmarkWindow;

use super::error::ReadApiError;

// the two reads the endpoints make: one aggregate query per window, one row per (key, session)
#[async_trait]
pub trait AutoRouterReadStore: Send + Sync {
    // AUTOROUTER_BENCHMARKS_SQL: the window's rows for one key or one canonical internal user
    async fn agg_rows(
        &self,
        window: &BenchmarkWindow,
        api_key: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<Vec<SessionAggRow>, ReadApiError>;

    // find_latest_for_key: the key is the row's own partition, never a filter over a wider read
    async fn latest_session(
        &self,
        api_key: &str,
        session_id: &str,
    ) -> Result<Option<AutoRouterSession>, ReadApiError>;
}
