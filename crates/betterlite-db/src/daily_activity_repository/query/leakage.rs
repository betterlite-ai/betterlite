use super::super::database::DailyActivityDatabase;
use super::super::rows::adapters::key_spend::key_spend_rows;
use super::super::{
    DailyActivityRepository, DailyActivityRepositoryError, Result, USAGE_CACHE_LEAKAGE_KEYS_MAX,
};
use crate::daily_activity_contracts::{DailyActivityProxyReads, DailyActivityScope, KeySpendRow};
use crate::daily_activity_export::build_cache_leakage_keys_sql;
use chrono::{DateTime, Utc};

impl<Database, Reads, Clock> DailyActivityRepository<Database, Reads, Clock>
where
    Database: DailyActivityDatabase,
    Reads: DailyActivityProxyReads,
    Clock: Fn() -> DateTime<Utc>,
{
    pub async fn cache_leakage_keys(
        &self,
        scope: &DailyActivityScope,
        limit: i64,
    ) -> Result<Vec<KeySpendRow>> {
        if !(1..=USAGE_CACHE_LEAKAGE_KEYS_MAX).contains(&limit) {
            return Err(DailyActivityRepositoryError::Limit {
                maximum: USAGE_CACHE_LEAKAGE_KEYS_MAX,
            });
        }
        let query = build_cache_leakage_keys_sql(scope, limit, &self.clock)?;
        let rows = self.query(&query).await?;
        Ok(key_spend_rows(&rows)?)
    }
}
