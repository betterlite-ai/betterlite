use crate::postgres_store::diagnostics::PoolFailure;
use crate::postgres_store::errors::{DbError, backend};
use betterlite_roi::records::{ROIReport, ROISyncPhase, ROISyncStatus};
use chrono::NaiveDateTime;
use deadpool_postgres::{Client, GenericClient, Pool};
use serde::{Deserialize, Serialize};
use tokio_postgres::types::Json;

type SyncResult<T> = Result<T, DbError<serde_json::Error>>;

const SYNC_KEY: &str = "roi_calculator_sync";
const REPORT_KEY: &str = "roi_calculator_report";
const EXPIRED_MESSAGE: &str =
    "The worker stopped responding. Run analysis again to resume saved estimates.";

const ACQUIRE_SQL: &str = r#"INSERT INTO "LiteLLM_Config" (param_name, param_value, last_run_at)
    VALUES ($1, $2::text::jsonb, NOW())
    ON CONFLICT (param_name) DO UPDATE
    SET param_value = EXCLUDED.param_value, last_run_at = NOW()
    WHERE ("LiteLLM_Config".last_run_at < NOW() - INTERVAL '60 seconds'
       OR "LiteLLM_Config".param_value->'status'->>'running' = 'false')
      AND ($3::text::double precision = 0 OR "LiteLLM_Config".last_run_at <= NOW() - $3::text::double precision * INTERVAL '1 minute')
    RETURNING param_name"#;

const FINISH_SQL: &str = r#"WITH owned AS (
        SELECT param_name FROM "LiteLLM_Config"
        WHERE param_name = $1 AND param_value->>'owner' = $2
          AND last_run_at >= NOW() - INTERVAL '60 seconds'
          AND ($4::jsonb IS NULL OR param_value->>'cancel' = 'false')
        FOR UPDATE
    ), report_write AS (
        INSERT INTO "LiteLLM_Config" (param_name, param_value)
        SELECT $5, $4::jsonb FROM owned WHERE $4::jsonb IS NOT NULL
        ON CONFLICT (param_name) DO UPDATE SET param_value = EXCLUDED.param_value
    ), cache_cleanup AS (
        DELETE FROM "LiteLLM_Config" cached
        WHERE starts_with(cached.param_name, 'roi_calculator_pull_')
          AND EXISTS (SELECT 1 FROM owned) AND $4::jsonb IS NOT NULL
          AND EXISTS (
              SELECT 1 FROM jsonb_array_elements($4::jsonb->'pulls') pull
              WHERE pull->>'url' = cached.param_value->>'url'
                AND pull->'estimate'->>'status' = 'estimated'
                AND pull->>'cache_key' IS NOT NULL
                AND cached.param_name <> 'roi_calculator_pull_' || (pull->>'cache_key')
          )
    )
    UPDATE "LiteLLM_Config" SET param_value = jsonb_set(param_value, '{status}', $3::jsonb),
        last_run_at = NOW()
    WHERE param_name IN (SELECT param_name FROM owned) RETURNING param_name"#;

const STATUS_SQL: &str = r#"SELECT param_value::text AS param_value, last_run_at,
        last_run_at < NOW() - INTERVAL '60 seconds' AS expired
    FROM "LiteLLM_Config" WHERE param_name = $1"#;

const HEARTBEAT_SQL: &str = r#"UPDATE "LiteLLM_Config"
    SET param_value = jsonb_set(param_value, '{status}', $3::jsonb), last_run_at = NOW()
    WHERE param_name = $1 AND param_value->>'owner' = $2
      AND param_value->>'cancel' = 'false'
      AND param_value->'status'->>'running' = 'true'
      AND last_run_at >= NOW() - INTERVAL '60 seconds'
    RETURNING param_name"#;

const CANCEL_SQL: &str = r#"UPDATE "LiteLLM_Config"
    SET param_value = param_value || jsonb_build_object(
        'cancel', true, 'owner', '',
        'status', (param_value->'status') || jsonb_build_object(
            'running', false, 'phase', 'cancelled', 'stage', 'Sync cancelled',
            'finished_at', to_char(NOW() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"+00:00"')
        )
    ), last_run_at = NOW()
    WHERE param_name = $1 AND param_value->'status'->>'running' = 'true'"#;

const CLEAR_REPORT_SQL: &str = r#"DELETE FROM "LiteLLM_Config" WHERE param_name = $1"#;

#[derive(Serialize, Deserialize)]
struct SyncState<Owner = String, Status = ROISyncStatus> {
    owner: Owner,
    status: Status,
    #[serde(default)]
    cancel: bool,
}

fn iso_format(instant: NaiveDateTime) -> String {
    if instant.and_utc().timestamp_subsec_micros() == 0 {
        instant.format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
    } else {
        instant.format("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string()
    }
}

pub struct FinishClaim<'claim> {
    pub owner: &'claim str,
    pub status: &'claim ROISyncStatus,
    pub report: Option<&'claim ROIReport>,
}

pub struct SyncStore<C = Client> {
    pub client: C,
}

impl SyncStore {
    pub async fn open(pool: &Pool) -> SyncResult<Self> {
        let client = pool.get().await.map_err(|driver| DbError::Pool {
            source: PoolFailure { driver },
        })?;
        Ok(Self { client })
    }
}

impl<C> SyncStore<C>
where
    C: GenericClient + Sync,
{
    pub async fn acquire(
        &self,
        owner: &str,
        status: &ROISyncStatus,
        scheduled_interval: f64,
    ) -> SyncResult<bool> {
        let state = SyncState {
            owner,
            status,
            cancel: false,
        };
        let encoded =
            serde_json::to_string(&state).map_err(|source| DbError::InvalidRecord { source })?;
        let interval = scheduled_interval.to_string();
        self.client
            .query_opt(ACQUIRE_SQL, &[&SYNC_KEY, &encoded, &interval])
            .await
            .map(|row| row.is_some())
            .map_err(backend)
    }

    pub async fn heartbeat(&self, owner: &str, status: &ROISyncStatus) -> SyncResult<bool> {
        Ok(self
            .client
            .execute(HEARTBEAT_SQL, &[&SYNC_KEY, &owner, &Json(status)])
            .await
            .map_err(backend)?
            > 0)
    }

    pub async fn finish(&self, claim: FinishClaim<'_>) -> SyncResult<bool> {
        let row = self
            .client
            .query_opt(
                FINISH_SQL,
                &[
                    &SYNC_KEY,
                    &claim.owner,
                    &Json(claim.status),
                    &claim.report.map(Json),
                    &REPORT_KEY,
                ],
            )
            .await
            .map_err(backend)?;
        Ok(row.is_some())
    }

    pub async fn status(&self) -> SyncResult<Option<ROISyncStatus>> {
        let Some(row) = self
            .client
            .query_opt(STATUS_SQL, &[&SYNC_KEY])
            .await
            .map_err(backend)?
        else {
            return Ok(None);
        };
        let encoded: String = row.get("param_value");
        let state: SyncState =
            serde_json::from_str(&encoded).map_err(|source| DbError::InvalidRecord { source })?;
        let expired: bool = row.get("expired");
        if expired && state.status.running {
            let last_run_at: NaiveDateTime = row.get("last_run_at");
            return Ok(Some(ROISyncStatus {
                running: false,
                phase: ROISyncPhase::Error,
                stage: "Sync interrupted".to_owned(),
                finished_at: Some(iso_format(last_run_at)),
                error: Some(EXPIRED_MESSAGE.to_owned()),
                ..state.status
            }));
        }
        Ok(Some(state.status))
    }

    pub async fn cancel(&self) -> SyncResult<()> {
        self.client
            .execute(CANCEL_SQL, &[&SYNC_KEY])
            .await
            .map(|_| ())
            .map_err(backend)
    }

    pub async fn clear_report(&self) -> SyncResult<()> {
        self.client
            .execute(CLEAR_REPORT_SQL, &[&REPORT_KEY])
            .await
            .map(|_| ())
            .map_err(backend)
    }
}

mod tests;
