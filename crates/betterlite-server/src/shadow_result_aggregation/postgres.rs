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
        let rows = self.rows(ATTEMPT_TOTALS_SQL, leg_ids).await?;
        let row = rows.first().ok_or(ShadowResultsError::NoTotalsRow)?;
        attempt_totals_row(row)
    }

    async fn attempt_count_rows(
        &self,
        leg_ids: &[String],
    ) -> RepositoryResult<Vec<LegAttemptCount>> {
        let rows = self.rows(ATTEMPT_COUNTS_SQL, leg_ids).await?;
        rows.iter().map(attempt_count_row).collect()
    }

    async fn funnel_totals(&self, leg_ids: &[String]) -> RepositoryResult<Option<FunnelTotals>> {
        self.rows(FUNNEL_TOTALS_SQL, leg_ids)
            .await?
            .first()
            .map(funnel_row)
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
        let client = self.store.session().await?;
        let params: [&(dyn ToSql + Sync); 1] = [&leg_ids];
        client
            .query(sql, &params)
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
        max_budget: max_budget
            .map(|value| {
                Decimal::from_f64_retain(value)
                    .ok_or(ShadowResultsError::Unrepresentable { name: "max_budget" })
            })
            .transpose()?,
        created_at: column::<NaiveDateTime>(row, "created_at")?.and_utc(),
        ends_at: column::<NaiveDateTime>(row, "ends_at")?.and_utc(),
        stopped_at: column::<Option<NaiveDateTime>>(row, "stopped_at")?
            .map(|stamp| stamp.and_utc()),
        stopped_by: column(row, "stopped_by")?,
    })
}

fn attempt_agg_row(row: &Row) -> RepositoryResult<AttemptAggRow> {
    Ok(AttemptAggRow {
        group: column(row, "grp")?,
        turn_count: i64::from(column::<i32>(row, "turn_count")?),
        real_wins: i64::from(column::<i32>(row, "real_wins")?),
        shadow_wins: i64::from(column::<i32>(row, "shadow_wins")?),
        ties: i64::from(column::<i32>(row, "ties")?),
        avg_confidence: column(row, "avg_confidence")?,
        real_spend: money(row, "real_spend")?,
        shadow_spend: money(row, "shadow_spend")?,
        cache_hit_turns: i64::from(column::<i32>(row, "cache_hit_turns")?),
    })
}

fn attempt_totals_row(row: &Row) -> RepositoryResult<AttemptTotals> {
    Ok(AttemptTotals {
        judged_count: i64::from(column::<i32>(row, "judged_count")?),
        error_count: i64::from(column::<i32>(row, "error_count")?),
        judge_spend: money(row, "judge_spend")?,
    })
}

fn attempt_count_row(row: &Row) -> RepositoryResult<LegAttemptCount> {
    Ok(LegAttemptCount {
        job_id: column(row, "job_id")?,
        attempt_count: i64::from(column::<i32>(row, "attempt_count")?),
        spend: money(row, "spend")?,
    })
}

fn funnel_row(row: &Row) -> RepositoryResult<FunnelTotals> {
    let legs_with_rows = i64::from(column::<i32>(row, "legs_with_rows")?);
    let not_sampled = i64::from(column::<i32>(row, "not_sampled")?);
    let unjudgeable = i64::from(column::<i32>(row, "unjudgeable")?);
    let shed = i64::from(column::<i32>(row, "shed")?);
    let withheld = i64::from(column::<i32>(row, "withheld")?);
    Ok(FunnelTotals {
        legs_with_rows,
        not_sampled,
        unjudgeable,
        shed,
        withheld,
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

// the column holds the SQL sum's double; retaining that exact value keeps Python's round of it
fn money(row: &Row, name: &'static str) -> RepositoryResult<Decimal> {
    let value: f64 = column(row, name)?;
    let exact = Decimal::from_f64_retain(value);
    exact.ok_or(ShadowResultsError::Unrepresentable { name })
}

fn column<T>(row: &Row, name: &'static str) -> RepositoryResult<T>
where
    T: for<'row> FromSql<'row>,
{
    row.try_get(name)
        .map_err(|source| ShadowResultsError::Database(backend(source)))
}
