use super::database::DailyActivityDatabase;
use super::rows::DatabaseRow;
use super::rows::adapters::entity::entity_rows;
use super::rows::adapters::grouping::grouping_rows;
use super::rows::adapters::key_page::key_page;
use super::rows::adapters::key_spend::key_spend_rows;
use super::{
    DailyActivityRepository, DailyActivityRepositoryError, Result, USAGE_CACHE_LEAKAGE_KEYS_MAX,
    USAGE_KEY_SEARCH_MAX, USAGE_MODEL_TOP_KEYS_MAX,
};
use crate::daily_activity_contracts::{
    AggregatedRows, DailyActivityProxyReads, DailyActivityScope, DailyRowsPage, KeyPage,
    KeySpendRow, SqlQuery,
};
use crate::daily_activity_export::{
    build_cache_leakage_keys_sql, build_key_search_sql, build_model_top_keys_sql,
};
use crate::daily_activity_sql::filters::build_where_clause;
use crate::daily_activity_sql::key_page::build_key_page_sql;
use crate::daily_activity_sql::rollup::{Rollup, build_rollup_sql};
use chrono::{DateTime, Utc};
use futures_util::future::join;

impl<Database, Reads, Clock> DailyActivityRepository<Database, Reads, Clock>
where
    Database: DailyActivityDatabase,
    Reads: DailyActivityProxyReads + Sync,
    Clock: Fn() -> DateTime<Utc> + Sync,
{
    pub(super) async fn query(&self, query: &SqlQuery) -> Result<Vec<DatabaseRow>> {
        let first_line = query.sql.trim_start().lines().next().unwrap_or_default();
        tracing::debug!(
            "DailyActivityRepository query: {}",
            first_line.trim_start_matches('(').trim()
        );
        let rows = self.database.query_raw(query).await?;
        Ok(rows.unwrap_or_default())
    }

    pub async fn aggregated(
        &self,
        scope: &DailyActivityScope,
        include_entity_breakdown: bool,
        api_key_limit: i64,
    ) -> Result<AggregatedRows> {
        let grouping = build_rollup_sql(scope, api_key_limit, Rollup::Aggregate, &self.clock)?;
        let entity_query = if include_entity_breakdown {
            Some(build_rollup_sql(
                scope,
                api_key_limit,
                Rollup::Entity,
                &self.clock,
            )?)
        } else {
            None
        };
        let pending_entity = async {
            match entity_query.as_ref() {
                Some(query) => self.query(query).await.map(Some),
                None => Ok(None),
            }
        };
        let (grouping_read, entity_read) = join(self.query(&grouping), pending_entity).await;
        let grouping_rows = grouping_rows(&grouping_read?)?;
        let entity_rows = match entity_read? {
            Some(found) => Some(entity_rows(&found)?),
            None => None,
        };
        let distinct_api_keys = grouping_rows
            .iter()
            .find_map(|row| row.distinct_api_keys)
            .unwrap_or(0);
        Ok(AggregatedRows {
            grouping_rows,
            entity_rows,
            distinct_api_keys,
        })
    }

    pub async fn key_page(
        &self,
        scope: &DailyActivityScope,
        offset: i64,
        limit: i64,
    ) -> Result<KeyPage> {
        let query = build_key_page_sql(scope, offset, limit, &self.clock)?;
        let rows = self.query(&query).await?;
        Ok(key_page(&rows)?)
    }

    pub async fn daily_rows(
        &self,
        scope: &DailyActivityScope,
        page: i64,
        page_size: i64,
    ) -> Result<DailyRowsPage> {
        let predicate = build_where_clause(scope, 1, &self.clock)?;
        let table = scope.as_ref().table;
        let skip = page
            .checked_sub(1)
            .and_then(|pages| pages.checked_mul(page_size))
            .filter(|skip| *skip >= 0)
            .ok_or(DailyActivityRepositoryError::Pagination)?;
        let (count, found) = join(
            self.database.count_daily(table, &predicate),
            self.database.find_daily(table, &predicate, skip, page_size),
        )
        .await;
        Ok(DailyRowsPage {
            total_count: count?,
            rows: found?,
        })
    }

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
