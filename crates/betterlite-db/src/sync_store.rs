use crate::postgres_store::errors::{DbError, backend};
use betterlite_roi::records::{ROIReport, ROISyncPhase, ROISyncStatus};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use tokio_postgres::GenericClient;
use tokio_postgres::types::ToSql;

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

const HEARTBEAT_SQL: &str = r#"UPDATE "LiteLLM_Config"
    SET param_value = jsonb_set(param_value, '{status}', $3::text::jsonb), last_run_at = NOW()
    WHERE param_name = $1 AND param_value->>'owner' = $2
      AND param_value->>'cancel' = 'false'
      AND param_value->'status'->>'running' = 'true'
      AND last_run_at >= NOW() - INTERVAL '60 seconds'
    RETURNING param_name"#;

const FINISH_SQL: &str = r#"WITH owned AS (
        SELECT param_name FROM "LiteLLM_Config"
        WHERE param_name = $1 AND param_value->>'owner' = $2
          AND last_run_at >= NOW() - INTERVAL '60 seconds'
          AND ($4::text IS NULL OR param_value->>'cancel' = 'false')
        FOR UPDATE
    ), report_write AS (
        INSERT INTO "LiteLLM_Config" (param_name, param_value)
        SELECT $5, $4::text::jsonb FROM owned WHERE $4::text IS NOT NULL
        ON CONFLICT (param_name) DO UPDATE SET param_value = EXCLUDED.param_value
    ), cache_cleanup AS (
        DELETE FROM "LiteLLM_Config" cached
        WHERE starts_with(cached.param_name, 'roi_calculator_pull_')
          AND EXISTS (SELECT 1 FROM owned) AND $4::text IS NOT NULL
          AND EXISTS (
              SELECT 1 FROM jsonb_array_elements($4::text::jsonb->'pulls') pull
              WHERE pull->>'url' = cached.param_value->>'url'
                AND pull->'estimate'->>'status' = 'estimated'
                AND pull->>'cache_key' IS NOT NULL
                AND cached.param_name <> 'roi_calculator_pull_' || (pull->>'cache_key')
          )
    )
    UPDATE "LiteLLM_Config" SET param_value = jsonb_set(param_value, '{status}', $3::text::jsonb),
        last_run_at = NOW()
    WHERE param_name IN (SELECT param_name FROM owned) RETURNING param_name"#;

const STATUS_SQL: &str = r#"SELECT param_value::text AS param_value, last_run_at,
        last_run_at < NOW() - INTERVAL '60 seconds' AS expired
    FROM "LiteLLM_Config" WHERE param_name = $1"#;

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
struct SyncState {
    owner: String,
    status: ROISyncStatus,
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

async fn update_lease<C>(
    client: &C,
    sql: &str,
    owner: &str,
    status: &ROISyncStatus,
    extra: &[&(dyn ToSql + Sync)],
) -> Result<bool, DbError<serde_json::Error>>
where
    C: GenericClient + Sync,
{
    let encoded = encode(status)?;
    let mut params: Vec<&(dyn ToSql + Sync)> = vec![&SYNC_KEY, &owner, &encoded];
    params.extend_from_slice(extra);
    let rows = client.query(sql, &params).await.map_err(backend)?;
    Ok(!rows.is_empty())
}

async fn discard<C>(client: &C, sql: &str, key: &str) -> Result<(), DbError<serde_json::Error>>
where
    C: GenericClient + Sync,
{
    client.execute(sql, &[&key]).await.map_err(backend)?;
    Ok(())
}

fn encode<T>(value: &T) -> Result<String, DbError<serde_json::Error>>
where
    T: Serialize,
{
    match serde_json::to_string(value) {
        Ok(encoded) => Ok(encoded),
        Err(source) => Err(DbError::InvalidRecord { source }),
    }
}

pub struct FinishClaim<'claim> {
    pub owner: &'claim str,
    pub status: &'claim ROISyncStatus,
    pub report: Option<&'claim ROIReport>,
}

pub struct SyncStore;

impl SyncStore {
    pub async fn acquire<C>(
        client: &C,
        owner: &str,
        status: &ROISyncStatus,
        scheduled_interval: f64,
    ) -> Result<bool, DbError<serde_json::Error>>
    where
        C: GenericClient + Sync,
    {
        let state = SyncState {
            owner: owner.to_owned(),
            status: status.clone(),
            cancel: false,
        };
        let encoded = encode(&state)?;
        let interval = scheduled_interval.to_string();
        let rows = client
            .query(ACQUIRE_SQL, &[&SYNC_KEY, &encoded, &interval])
            .await
            .map_err(backend)?;
        Ok(!rows.is_empty())
    }

    pub async fn heartbeat<C>(
        client: &C,
        owner: &str,
        status: &ROISyncStatus,
    ) -> Result<bool, DbError<serde_json::Error>>
    where
        C: GenericClient + Sync,
    {
        let affected = client
            .execute(HEARTBEAT_SQL, &[&SYNC_KEY, &owner, &encode(status)?])
            .await
            .map_err(backend)?;
        Ok(affected > 0)
    }

    pub async fn finish<C>(
        client: &C,
        claim: FinishClaim<'_>,
    ) -> Result<bool, DbError<serde_json::Error>>
    where
        C: GenericClient + Sync,
    {
        let FinishClaim {
            owner,
            status,
            report,
        } = claim;
        let encoded_report = report.map(encode).transpose()?;
        update_lease(
            client,
            FINISH_SQL,
            owner,
            status,
            &[&encoded_report, &REPORT_KEY],
        )
        .await
    }

    pub async fn status<C>(client: &C) -> Result<Option<ROISyncStatus>, DbError<serde_json::Error>>
    where
        C: GenericClient + Sync,
    {
        let Some(row) = client
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

    pub async fn cancel<C>(client: &C) -> Result<(), DbError<serde_json::Error>>
    where
        C: GenericClient + Sync,
    {
        discard(client, CANCEL_SQL, SYNC_KEY).await?;
        Ok(())
    }

    pub async fn clear_report<C>(client: &C) -> Result<(), DbError<serde_json::Error>>
    where
        C: GenericClient + Sync,
    {
        discard(client, CLEAR_REPORT_SQL, REPORT_KEY).await?;
        Ok(())
    }
}

mod tests;
