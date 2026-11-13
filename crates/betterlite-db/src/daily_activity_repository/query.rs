use super::database::{DailyActivityDatabase, DailyRowFilter};
use super::rows::{self, DatabaseRow};
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
use crate::daily_activity_sql::filters::adjust_dates_for_timezone;
use crate::daily_activity_sql::key_page::build_key_page_sql;
use crate::daily_activity_sql::rollup::{Rollup, build_rollup_sql};
use chrono::{DateTime, Utc};
use futures_util::future::join;

impl<Database, Reads, Clock> DailyActivityRepository<Database, Reads, Clock>
where
    Database: DailyActivityDatabase,
    Reads: DailyActivityProxyReads,
    Clock: Fn() -> DateTime<Utc>,
{
    pub(super) async fn query(&self, query: &SqlQuery) -> Result<Vec<DatabaseRow>> {
        tracing::debug!(
            "DailyActivityRepository query: {}",
            first_sql_line(&query.sql)
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
        let entity_read = async {
            match entity_query.as_ref() {
                Some(query) => self.query(query).await.map(Some),
                None => Ok(None),
            }
        };
        let (grouping_read, entity_read) = join(self.query(&grouping), entity_read).await;
        let grouping_rows = rows::grouping_rows(&grouping_read?)?;
        let entity_rows = match entity_read? {
            Some(found) => Some(rows::entity_rows(&found)?),
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
        let decoded = rows::key_spend_rows(&rows)?;
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
        Ok(rows::key_spend_rows(&rows)?)
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
        Ok(rows::key_spend_rows(&rows)?)
    }

    pub async fn key_page(
        &self,
        scope: &DailyActivityScope,
        offset: i64,
        limit: i64,
    ) -> Result<KeyPage> {
        let query = build_key_page_sql(scope, offset, limit, &self.clock)?;
        let rows = self.query(&query).await?;
        Ok(rows::key_page(&rows)?)
    }

    pub async fn daily_rows(
        &self,
        scope: &DailyActivityScope,
        page: i64,
        page_size: i64,
    ) -> Result<DailyRowsPage> {
        let input = scope.as_ref();
        let (start, end) = adjust_dates_for_timezone(
            &input.start_date,
            &input.end_date,
            input.timezone_offset_minutes,
            input.include_current_utc_day,
            &self.clock,
        )?;
        let mut filters = vec![DailyRowFilter::DateRange { start, end }];
        if let Some(entity_ids) = input.entity_ids.as_ref() {
            filters.push(DailyRowFilter::EntityIn {
                values: entity_ids.clone(),
            });
        }
        if !input.exclude_entity_ids.is_empty() {
            filters.push(DailyRowFilter::EntityNotIn {
                values: input.exclude_entity_ids.clone(),
            });
        }
        if let Some(model) = input.model.as_deref().filter(|model| !model.is_empty()) {
            filters.push(DailyRowFilter::Model {
                value: model.to_owned(),
            });
        }
        if let Some(api_keys) = input.api_keys.as_ref() {
            filters.push(DailyRowFilter::ApiKeyIn {
                values: api_keys.clone(),
            });
        }
        let skip = page
            .checked_sub(1)
            .and_then(|pages| pages.checked_mul(page_size))
            .filter(|skip| *skip >= 0)
            .ok_or(DailyActivityRepositoryError::Pagination)?;
        let (count, found) = join(
            self.database.count_daily(input.table, &filters),
            self.database
                .find_daily(input.table, &filters, skip, page_size),
        )
        .await;
        Ok(DailyRowsPage {
            total_count: count?,
            rows: found?,
        })
    }
}

fn first_sql_line(sql: &str) -> &str {
    let line = sql.trim_start().lines().next().unwrap_or_default();
    line.trim_start_matches('(').trim()
}
