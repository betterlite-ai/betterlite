use super::super::database::DailyActivityDatabase;
use super::super::rows::adapters::key_spend::key_spend_rows;
use super::super::{
    DailyActivityRepository, DailyActivityRepositoryError, Result, USAGE_MODEL_TOP_KEYS_MAX,
};
use crate::daily_activity_contracts::{DailyActivityProxyReads, DailyActivityScope, KeySpendRow};
use crate::daily_activity_export::build_model_top_keys_sql;
use chrono::{DateTime, Utc};

impl<Database, Reads, Clock> DailyActivityRepository<Database, Reads, Clock>
where
    Database: DailyActivityDatabase,
    Reads: DailyActivityProxyReads,
    Clock: Fn() -> DateTime<Utc>,
{
    pub async fn model_top_keys(
        &self,
        scope: &DailyActivityScope,
        model_group: &str,
        by_model_group: bool,
        limit: i64,
    ) -> Result<Vec<KeySpendRow>> {
        if !(1..=USAGE_MODEL_TOP_KEYS_MAX).contains(&limit) {
            return Err(DailyActivityRepositoryError::Limit {
                maximum: USAGE_MODEL_TOP_KEYS_MAX,
            });
        }
        let query =
            build_model_top_keys_sql(scope, model_group, by_model_group, limit, &self.clock)?;
        let rows = self.query(&query).await?;
        Ok(key_spend_rows(&rows)?)
    }
}
