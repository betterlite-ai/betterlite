use chrono::NaiveDateTime;
use deadpool_postgres::GenericClient as _;
use rust_decimal::Decimal;
use tokio_postgres::types::ToSql;

use crate::postgres_store::session::PostgresStore;

use super::error::{WindowSpendError, database};
use super::seed::{
    SeedEntity, SeedQuery, WindowSeedTotals, WindowSpendLogs, exclusion_upper_bound,
};

pub(super) const EXISTING_ROWS_SQL: &str = r#"SELECT entity_type, entity_id, window_duration FROM "LiteLLM_BudgetWindowSpend" WHERE (entity_type, entity_id, window_duration) IN (SELECT * FROM unnest($1::text[], $2::text[], $3::text[]))"#;

// $5 is what a new row starts at and $6 the increment alone, so two pods seeding one new window never add the seed twice
pub(super) const UPSERT_WINDOW_SPEND_SQL: &str = r#"INSERT INTO "LiteLLM_BudgetWindowSpend" (entity_type, entity_id, window_duration, window_start, spend, created_at, updated_at) VALUES ($1, $2, $3, $4, $5::numeric::double precision, $7, $7) ON CONFLICT (entity_type, entity_id, window_duration) DO UPDATE SET spend = CASE WHEN "LiteLLM_BudgetWindowSpend".window_start >= EXCLUDED.window_start THEN "LiteLLM_BudgetWindowSpend".spend + $6::numeric::double precision ELSE EXCLUDED.spend END, window_start = GREATEST("LiteLLM_BudgetWindowSpend".window_start, EXCLUDED.window_start), updated_at = $7"#;

pub(super) const ROLL_WINDOW_SPEND_SQL: &str = r#"UPDATE "LiteLLM_BudgetWindowSpend" SET window_start = $4, spend = 0, updated_at = $5 WHERE entity_type = $1 AND entity_id = $2 AND window_duration = $3 AND window_start < $4"#;

// Without a batch start $3 is null, so both sums are the whole total as _SEED_FROM_SPEND_LOGS_*_UNBOUNDED_SQL makes them
const KEY_SEED_SQL: &str = r#"SELECT COALESCE(SUM(spend), 0.0)::numeric AS total, COALESCE(SUM(spend) FILTER (WHERE $3::timestamp IS NULL OR "startTime" < $3::timestamp), 0.0)::numeric AS before_batch FROM "LiteLLM_SpendLogs" WHERE api_key = $1 AND "startTime" >= $2::timestamp"#;
const TEAM_SEED_SQL: &str = r#"SELECT COALESCE(SUM(spend), 0.0)::numeric AS total, COALESCE(SUM(spend) FILTER (WHERE $3::timestamp IS NULL OR "startTime" < $3::timestamp), 0.0)::numeric AS before_batch FROM "LiteLLM_SpendLogs" WHERE team_id = $1 AND "startTime" >= $2::timestamp"#;

pub struct SqlWindowSpendLogs {
    pub store: PostgresStore,
}

impl WindowSpendLogs for SqlWindowSpendLogs {
    async fn seed_totals(
        &self,
        query: &SeedQuery<'_>,
    ) -> Result<WindowSeedTotals, WindowSpendError> {
        let statement = match query.entity {
            SeedEntity::Key => KEY_SEED_SQL,
            SeedEntity::Team => TEAM_SEED_SQL,
        };
        let bound: Option<NaiveDateTime> = query.batch_started_at.map(exclusion_upper_bound);
        let values: [&(dyn ToSql + Sync); 3] = [&query.entity_id, &query.window_start, &bound];
        let client = self.store.session().await?;
        let row = client
            .query_one(statement, &values)
            .await
            .map_err(database)?;
        let total: Decimal = row.try_get(0).map_err(database)?;
        let before_batch: Decimal = row.try_get(1).map_err(database)?;
        Ok(WindowSeedTotals {
            total,
            before_batch,
        })
    }
}
