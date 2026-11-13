use super::super::database::DailyActivityDatabase;
use super::super::rows::adapters::key_spend::key_spend_rows;
use super::super::{
    DailyActivityRepository, DailyActivityRepositoryError, Result, USAGE_KEY_SEARCH_MAX,
};
use crate::daily_activity_contracts::{DailyActivityProxyReads, DailyActivityScope};
use crate::daily_activity_export::build_key_search_sql;
use chrono::{DateTime, Utc};

impl<Database, Reads, Clock> DailyActivityRepository<Database, Reads, Clock>
where
    Database: DailyActivityDatabase,
    Reads: DailyActivityProxyReads,
    Clock: Fn() -> DateTime<Utc>,
{
    pub async fn search_keys(
        &self,
        scope: &DailyActivityScope,
        search: &str,
        limit: i64,
    ) -> Result<Vec<String>> {
        if !(1..=USAGE_KEY_SEARCH_MAX).contains(&limit) {
            return Err(DailyActivityRepositoryError::Limit {
                maximum: USAGE_KEY_SEARCH_MAX,
            });
        }
        let query = build_key_search_sql(scope, search, limit, &self.clock)?;
        let rows = self.query(&query).await?;
        let decoded = key_spend_rows(&rows)?;
        Ok(decoded.into_iter().map(|row| row.api_key).collect())
    }
}
