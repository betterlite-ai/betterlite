pub mod leakage;
pub mod models;
pub mod search;

use super::database::{DailyActivityDatabase, DailyRowFilter};
use super::rows::DatabaseRow;
use super::rows::adapters::entity::entity_rows;
use super::rows::adapters::grouping::grouping_rows;
use super::rows::adapters::key_page::key_page;
use super::{DailyActivityRepository, DailyActivityRepositoryError, Result};
use crate::daily_activity_contracts::{
    AggregatedRows, DailyActivityProxyReads, DailyActivityScope, DailyRowsPage, KeyPage, SqlQuery,
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
        let entity_read = async {
            match entity_query.as_ref() {
                Some(query) => self.query(query).await.map(Some),
                None => Ok(None),
            }
        };
        let (grouping_read, entity_read) = join(self.query(&grouping), entity_read).await;
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
        let input = scope.as_ref();
        let (start, end) = adjust_dates_for_timezone(
            &input.start_date,
            &input.end_date,
            input.timezone_offset_minutes,
            input.include_current_utc_day,
            &self.clock,
        )?;
        let mut filters = vec![DailyRowFilter::DateRange { start, end }];
        if let Some(values) = input.entity_ids.as_ref() {
            filters.push(DailyRowFilter::EntityIn {
                values: values.clone(),
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
        if let Some(values) = input.api_keys.as_ref() {
            filters.push(DailyRowFilter::ApiKeyIn {
                values: values.clone(),
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
