use async_trait::async_trait;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use tokio_postgres::Row;
use tokio_postgres::types::{FromSql, ToSql};

use super::aggregation::{AttemptAggRow, GroupBy};
use super::error::ShadowResultsError;
use super::labels::{LabelKind, LabelRow};
use super::legs::LegRow;
use super::queries::{
    ATTEMPT_AGG_BY_LEG_SQL, ATTEMPT_AGG_BY_MODEL_SQL, ATTEMPT_AGG_BY_ROUTER_SQL,
    ATTEMPT_AGG_BY_TIER_SQL, ATTEMPT_COUNTS_SQL, ATTEMPT_TOTALS_SQL, FUNNEL_TOTALS_SQL,
    KEY_LABELS_SQL, LATEST_ERROR_ROWS_SQL, TEAM_LABELS_SQL, USER_LABELS_SQL,
};
use super::repository::{RepositoryResult, ShadowResultsRepository};
use super::responses::{AttemptTotals, ErrorAttempt, LegAttemptCount, latest_error};
use super::results::FunnelTotals;
use super::targets::parse_target_type;
use crate::shadow_eval_job_loading::jobs::ShadowEvalDirection;

pub struct PgShadowResults {
    pub store: PostgresStore,
}

#[async_trait]
impl ShadowResultsRepository for PgShadowResults {
    async fn attempt_aggregates(
        &self,
        leg_ids: &[String],
        group_by: GroupBy,
    ) -> RepositoryResult<Vec<AttemptAggRow>> {
        let sql = match group_by {
            GroupBy::Tier => ATTEMPT_AGG_BY_TIER_SQL,
            GroupBy::Model => ATTEMPT_AGG_BY_MODEL_SQL,
            GroupBy::Leg => ATTEMPT_AGG_BY_LEG_SQL,
            GroupBy::Router => ATTEMPT_AGG_BY_ROUTER_SQL,
        };
        let rows = self.rows(sql, leg_ids).await?;
        rows.iter().map(attempt_agg_row).collect()
    }

    async fn attempt_totals(&self, leg_ids: &[String]) -> RepositoryResult<AttemptTotals> {
        let row = self.one(ATTEMPT_TOTALS_SQL, leg_ids).await?;
        attempt_totals_row(&row)
    }

    async fn attempt_count_rows(
        &self,
        leg_ids: &[String],
    ) -> RepositoryResult<Vec<LegAttemptCount>> {
        let rows = self.rows(ATTEMPT_COUNTS_SQL, leg_ids).await?;
        rows.iter().map(attempt_count_row).collect()
    }

    async fn funnel_totals(&self, leg_ids: &[String]) -> RepositoryResult<Option<FunnelTotals>> {
        self.optional(FUNNEL_TOTALS_SQL, leg_ids)
            .await?
            .map(|row| funnel_row(&row))
            .transpose()
    }

    async fn latest_error(&self, leg_ids: &[String]) -> RepositoryResult<Option<String>> {
        let rows = self.rows(LATEST_ERROR_ROWS_SQL, leg_ids).await?;
        let attempts = rows
            .iter()
            .map(error_row)
            .collect::<RepositoryResult<Vec<_>>>()?;
        Ok(latest_error(&attempts))
    }

    async fn label_rows(
        &self,
        kind: LabelKind,
        target_ids: &[String],
    ) -> RepositoryResult<Vec<LabelRow>> {
        let (sql, decode): (&str, fn(&Row) -> RepositoryResult<LabelRow>) = match kind {
            LabelKind::Key => (KEY_LABELS_SQL, key_label_row),
            LabelKind::Team => (TEAM_LABELS_SQL, team_label_row),
            LabelKind::User => (USER_LABELS_SQL, user_label_row),
        };
        let rows = self.rows(sql, target_ids).await?;
        rows.iter().map(decode).collect()
    }
}

impl PgShadowResults {
    async fn rows(&self, sql: &str, leg_ids: &[String]) -> RepositoryResult<Vec<Row>> {
        let ids = leg_ids.to_vec();
        let client = self.store.session().await?;
        let params: [&(dyn ToSql + Sync); 1] = [&ids];
        client
            .query(sql, &params)
            .await
            .map_err(|source| ShadowResultsError::Database(backend(source)))
    }

    async fn one(&self, sql: &str, leg_ids: &[String]) -> RepositoryResult<Row> {
        let ids = leg_ids.to_vec();
        let client = self.store.session().await?;
        let params: [&(dyn ToSql + Sync); 1] = [&ids];
        client
            .query_one(sql, &params)
            .await
            .map_err(|source| ShadowResultsError::Database(backend(source)))
    }

    async fn optional(&self, sql: &str, leg_ids: &[String]) -> RepositoryResult<Option<Row>> {
        let ids = leg_ids.to_vec();
        let client = self.store.session().await?;
        let params: [&(dyn ToSql + Sync); 1] = [&ids];
        client
            .query_opt(sql, &params)
            .await
            .map_err(|source| ShadowResultsError::Database(backend(source)))
    }
}

// one LiteLLM_ShadowEvalJob row selected by a caller's own query, with every column validated
pub fn leg_row(row: &Row) -> RepositoryResult<LegRow> {
    let target_type: String = column(row, "target_type")?;
    let direction: String = column(row, "direction")?;
    let max_turns: i32 = column(row, "max_turns")?;
    let max_budget: Option<f64> = column(row, "max_budget")?;
    let max_budget = match max_budget {
        Some(value) => Some(
            Decimal::from_f64_retain(value)
                .ok_or(ShadowResultsError::Unrepresentable { name: "max_budget" })?,
        ),
        None => None,
    };
    Ok(LegRow {
        id: column(row, "id")?,
        group_id: column(row, "group_id")?,
        target_type: parse_target_type(&target_type)?,
        target_id: column(row, "target_id")?,
        router_name: column(row, "router_name")?,
        router_names: column(row, "router_names")?,
        models: column(row, "models")?,
        direction: ShadowEvalDirection::parse(&direction)?,
        baseline_model: column(row, "baseline_model")?,
        judge_model: column(row, "judge_model")?,
        shadow_percentage: column(row, "shadow_percentage")?,
        max_turns: i64::from(max_turns),
        max_budget,
        created_at: column::<NaiveDateTime>(row, "created_at")?.and_utc(),
        ends_at: column::<NaiveDateTime>(row, "ends_at")?.and_utc(),
        stopped_at: column::<Option<NaiveDateTime>>(row, "stopped_at")?.map(NaiveDateTime::and_utc),
        stopped_by: column(row, "stopped_by")?,
    })
}

fn attempt_agg_row(row: &Row) -> RepositoryResult<AttemptAggRow> {
    let turn_count: i32 = column(row, "turn_count")?;
    let real_wins: i32 = column(row, "real_wins")?;
    let shadow_wins: i32 = column(row, "shadow_wins")?;
    let ties: i32 = column(row, "ties")?;
    let cache_hit_turns: i32 = column(row, "cache_hit_turns")?;
    Ok(AttemptAggRow {
        group: column(row, "grp")?,
        turn_count: i64::from(turn_count),
        real_wins: i64::from(real_wins),
        shadow_wins: i64::from(shadow_wins),
        ties: i64::from(ties),
        avg_confidence: column(row, "avg_confidence")?,
        real_spend: column(row, "real_spend")?,
        shadow_spend: column(row, "shadow_spend")?,
        cache_hit_turns: i64::from(cache_hit_turns),
    })
}

fn attempt_totals_row(row: &Row) -> RepositoryResult<AttemptTotals> {
    let judged_count: i32 = column(row, "judged_count")?;
    let error_count: i32 = column(row, "error_count")?;
    Ok(AttemptTotals {
        judged_count: i64::from(judged_count),
        error_count: i64::from(error_count),
        judge_spend: column(row, "judge_spend")?,
    })
}

fn attempt_count_row(row: &Row) -> RepositoryResult<LegAttemptCount> {
    let attempt_count: i32 = column(row, "attempt_count")?;
    Ok(LegAttemptCount {
        job_id: column(row, "job_id")?,
        attempt_count: i64::from(attempt_count),
        spend: column(row, "spend")?,
    })
}

fn funnel_row(row: &Row) -> RepositoryResult<FunnelTotals> {
    let legs_with_rows: i32 = column(row, "legs_with_rows")?;
    let not_sampled: i32 = column(row, "not_sampled")?;
    let unjudgeable: i32 = column(row, "unjudgeable")?;
    let shed: i32 = column(row, "shed")?;
    let withheld: i32 = column(row, "withheld")?;
    Ok(FunnelTotals {
        legs_with_rows: i64::from(legs_with_rows),
        not_sampled: i64::from(not_sampled),
        unjudgeable: i64::from(unjudgeable),
        shed: i64::from(shed),
        withheld: i64::from(withheld),
    })
}

fn error_row(row: &Row) -> RepositoryResult<ErrorAttempt> {
    Ok(ErrorAttempt {
        error: column(row, "error")?,
        created_at: column::<NaiveDateTime>(row, "created_at")?.and_utc(),
    })
}

fn key_label_row(row: &Row) -> RepositoryResult<LabelRow> {
    Ok(LabelRow {
        target_id: column(row, "target_id")?,
        target_alias: column(row, "key_alias")?,
        key_name: column(row, "key_name")?,
    })
}

fn team_label_row(row: &Row) -> RepositoryResult<LabelRow> {
    Ok(LabelRow {
        target_id: column(row, "target_id")?,
        target_alias: column(row, "team_alias")?,
        key_name: None,
    })
}

fn user_label_row(row: &Row) -> RepositoryResult<LabelRow> {
    Ok(LabelRow {
        target_id: column(row, "target_id")?,
        target_alias: column(row, "user_email")?,
        key_name: None,
    })
}

fn column<T>(row: &Row, name: &'static str) -> RepositoryResult<T>
where
    T: for<'row> FromSql<'row>,
{
    row.try_get(name)
        .map_err(|source| ShadowResultsError::Database(backend(source)))
}
