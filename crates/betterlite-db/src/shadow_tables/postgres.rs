use chrono::{DateTime, NaiveDateTime, Utc};
use deadpool_postgres::GenericClient;
use rust_decimal::Decimal;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;
use uuid::Uuid;

use super::rows::{
    FunnelCounters, JobAttemptStats, ShadowAttemptRow, ShadowFuture, ShadowJobRow, ShadowTables,
};

const ACTIVE_JOBS_SQL: &str = r#"SELECT "id", "target_type", "target_id", "router_name",
       "router_names", "models", "direction", "baseline_model", "shadow_percentage",
       "judge_model", "max_turns"::bigint AS "max_turns",
       "max_budget"::numeric AS "max_budget", "ends_at"
FROM "LiteLLM_ShadowEvalJob"
WHERE "stopped_at" IS NULL AND "ends_at" > $1"#;

const ATTEMPT_STATS_SQL: &str = r#"SELECT "job_id",
       COUNT(*)::bigint AS "attempts",
       (COALESCE(SUM("judge_cost"), 0) + COALESCE(SUM("shadow_cost"), 0)
        + COALESCE(SUM("shadow_classifier_cost"), 0))::numeric AS "spend"
FROM "LiteLLM_ShadowEvalAttempt"
WHERE "job_id" = ANY($1)
GROUP BY "job_id""#;

const INSERT_ATTEMPT_SQL: &str = r#"INSERT INTO "LiteLLM_ShadowEvalAttempt"
    ("id", "job_id", "request_id", "router_name", "outcome", "tier", "real_model",
     "shadow_model", "confidence", "judge_cost", "shadow_cost", "shadow_classifier_cost",
     "real_cost", "real_classifier_cost", "real_cache_hit", "error")
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::numeric::float8, $10::numeric::float8,
        $11::numeric::float8, $12::numeric::float8, $13::numeric::float8,
        $14::numeric::float8, $15, $16)"#;

const UPSERT_FUNNEL_SQL: &str = r#"INSERT INTO "LiteLLM_ShadowEvalFunnel"
    ("job_id", "not_sampled", "unjudgeable", "shed", "withheld")
VALUES ($1, $2, $3, $4, $5)
ON CONFLICT ("job_id") DO UPDATE SET
    "not_sampled" = "LiteLLM_ShadowEvalFunnel"."not_sampled" + EXCLUDED."not_sampled",
    "unjudgeable" = "LiteLLM_ShadowEvalFunnel"."unjudgeable" + EXCLUDED."unjudgeable",
    "shed" = "LiteLLM_ShadowEvalFunnel"."shed" + EXCLUDED."shed",
    "withheld" = "LiteLLM_ShadowEvalFunnel"."withheld" + EXCLUDED."withheld""#;

pub struct PostgresShadowTables<Client> {
    pub client: Client,
}

impl<Client> ShadowTables for PostgresShadowTables<Client>
where
    Client: GenericClient + Send + Sync,
{
    type Error = tokio_postgres::Error;

    fn find_active_jobs(
        &self,
        now: DateTime<Utc>,
    ) -> ShadowFuture<'_, Vec<ShadowJobRow>, Self::Error> {
        Box::pin(async move {
            let naive = now.naive_utc();
            let rows = self.client.query(ACTIVE_JOBS_SQL, &[&naive]).await?;
            rows.iter().map(active_job_row).collect()
        })
    }

    fn attempt_stats<'database>(
        &'database self,
        job_ids: &'database [String],
    ) -> ShadowFuture<'database, Vec<JobAttemptStats>, Self::Error> {
        Box::pin(async move {
            let ids = job_ids.to_vec();
            let rows = self.client.query(ATTEMPT_STATS_SQL, &[&ids]).await?;
            rows.iter().map(attempt_stats_row).collect()
        })
    }

    fn insert_attempt<'database>(
        &'database self,
        attempt: &'database ShadowAttemptRow,
    ) -> ShadowFuture<'database, (), Self::Error> {
        Box::pin(async move {
            let id = Uuid::new_v4().to_string();
            let outcome = attempt.outcome.as_str();
            let parameters: [&(dyn ToSql + Sync); 16] = [
                &id,
                &attempt.job_id,
                &attempt.request_id,
                &attempt.router_name,
                &outcome,
                &attempt.tier,
                &attempt.real_model,
                &attempt.shadow_model,
                &attempt.confidence,
                &attempt.judge_cost,
                &attempt.shadow_cost,
                &attempt.shadow_classifier_cost,
                &attempt.real_cost,
                &attempt.real_classifier_cost,
                &attempt.real_cache_hit,
                &attempt.error,
            ];
            self.client
                .execute_raw(INSERT_ATTEMPT_SQL, parameters)
                .await?;
            Ok(())
        })
    }

    fn upsert_funnel<'database>(
        &'database self,
        job_id: &'database str,
        counters: FunnelCounters,
    ) -> ShadowFuture<'database, (), Self::Error> {
        Box::pin(async move {
            let parameters: [&(dyn ToSql + Sync); 5] = [
                &job_id,
                &counters.not_sampled,
                &counters.unjudgeable,
                &counters.shed,
                &counters.withheld,
            ];
            self.client
                .execute_raw(UPSERT_FUNNEL_SQL, parameters)
                .await?;
            Ok(())
        })
    }
}

fn active_job_row(row: &Row) -> Result<ShadowJobRow, tokio_postgres::Error> {
    Ok(ShadowJobRow {
        id: row.try_get("id")?,
        target_type: row.try_get("target_type")?,
        target_id: row.try_get("target_id")?,
        router_name: row.try_get("router_name")?,
        router_names: row.try_get("router_names")?,
        models: row.try_get("models")?,
        direction: row.try_get("direction")?,
        baseline_model: row.try_get("baseline_model")?,
        shadow_percentage: row.try_get("shadow_percentage")?,
        judge_model: row.try_get("judge_model")?,
        max_turns: row.try_get("max_turns")?,
        max_budget: row.try_get("max_budget")?,
        ends_at: row.try_get::<_, NaiveDateTime>("ends_at")?.and_utc(),
    })
}

fn attempt_stats_row(row: &Row) -> Result<JobAttemptStats, tokio_postgres::Error> {
    Ok(JobAttemptStats {
        job_id: row.try_get("job_id")?,
        attempts: row.try_get("attempts")?,
        spend: row.try_get::<_, Decimal>("spend")?,
    })
}
