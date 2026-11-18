use betterlite_redis::config_sync::table_actions::TableActions as _;
use chrono::{DateTime, Utc};
use deadpool_postgres::GenericClient as _;
use serde::Serialize;

use crate::auxiliary_repository::repository::{PrismaClient, TableError};
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::{DbError, backend};
use crate::resource_repository::config::{ConfigRepository, ConfigUpdate, ConfigWrite};
use crate::resource_repository::table::KeyFilter;
use crate::search_tools::create::dump::safe_dumps;

use super::RELOAD_CONFIG_PARAM_NAME;
use super::ReloadSchedule;
use super::evictor::ConfigParamEvictor;
use super::parse_schedule::{ReloadConfigRow, parse_reload_schedule};

// update_many without an upsert: a row deleted mid-poll stays deleted and the update affects nothing
const RECORD_RUN_SQL: &str =
    r#"UPDATE "LiteLLM_Config" SET "last_run_at" = $2 WHERE "param_name" = $1"#;

// the create starts the counter at 1; the update increments the stored one, which Postgres serializes per row
const RECORD_MANUAL_RELOAD_SQL: &str = r#"INSERT INTO "LiteLLM_Config" ("param_name", "last_run_at", "reload_revision")
VALUES ($1, $2, 1)
ON CONFLICT ("param_name") DO UPDATE
SET "last_run_at" = EXCLUDED."last_run_at",
    "reload_revision" = "LiteLLM_Config"."reload_revision" + 1
RETURNING "reload_revision""#;

#[derive(Debug, thiserror::Error)]
pub enum ReloadScheduleError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Table(#[from] TableError),
    #[error("the reload schedule value cannot be serialized")]
    Encode(#[source] serde_json::Error),
    #[error("the published reload revision {value} does not fit a u32")]
    Revision { value: i64 },
}

#[derive(Serialize)]
struct IntervalConfig {
    interval_hours: Option<u32>,
}

pub async fn read_reload_schedule(
    client: Option<PrismaClient<'_>>,
) -> Result<Option<ReloadSchedule>, ReloadScheduleError> {
    let rows = ConfigRepository::new(client, false)
        .table()?
        .as_ref()
        .find::<ReloadConfigRow>(KeyFilter::Equal(RELOAD_CONFIG_PARAM_NAME))
        .await?;
    Ok(rows.first().map(parse_reload_schedule))
}

pub async fn write_reload_interval(
    client: Option<PrismaClient<'_>>,
    evictor: &dyn ConfigParamEvictor,
    interval_hours: u32,
) -> Result<(), ReloadScheduleError> {
    let param_value = safe_dumps(&IntervalConfig {
        interval_hours: Some(interval_hours),
    })
    .map_err(ReloadScheduleError::Encode)?;
    let write = ConfigWrite {
        param_name: RELOAD_CONFIG_PARAM_NAME.to_owned(),
        param_value: JsonData::String(param_value),
    };
    ConfigRepository::new(client, true)
        .table()?
        .upsert(RELOAD_CONFIG_PARAM_NAME.to_owned(), write)
        .await?;
    evictor.evict_param(RELOAD_CONFIG_PARAM_NAME).await;
    Ok(())
}

pub async fn clear_reload_interval(
    client: Option<PrismaClient<'_>>,
    evictor: &dyn ConfigParamEvictor,
) -> Result<(), ReloadScheduleError> {
    let param_value = safe_dumps(&IntervalConfig {
        interval_hours: None,
    })
    .map_err(ReloadScheduleError::Encode)?;
    let update = ConfigUpdate {
        param_value: Some(JsonData::String(param_value)),
    };
    ConfigRepository::new(client, true)
        .table()?
        .update_many(update, RELOAD_CONFIG_PARAM_NAME.to_owned())
        .await?;
    evictor.evict_param(RELOAD_CONFIG_PARAM_NAME).await;
    Ok(())
}

pub async fn record_reload_run(
    client: Option<PrismaClient<'_>>,
    evictor: &dyn ConfigParamEvictor,
    ran_at: DateTime<Utc>,
) -> Result<(), ReloadScheduleError> {
    let store = ConfigRepository::new(client, true).prisma_client()?.writer;
    let session = store.session().await?;
    session
        .execute(
            RECORD_RUN_SQL,
            &[&RELOAD_CONFIG_PARAM_NAME, &ran_at.naive_utc()],
        )
        .await
        .map_err(|source| ReloadScheduleError::Database(backend(source)))?;
    evictor.evict_param(RELOAD_CONFIG_PARAM_NAME).await;
    Ok(())
}

pub async fn record_manual_reload(
    client: Option<PrismaClient<'_>>,
    evictor: &dyn ConfigParamEvictor,
    ran_at: DateTime<Utc>,
) -> Result<u32, ReloadScheduleError> {
    let store = ConfigRepository::new(client, true).prisma_client()?.writer;
    let session = store.session().await?;
    let row = session
        .query_one(
            RECORD_MANUAL_RELOAD_SQL,
            &[&RELOAD_CONFIG_PARAM_NAME, &ran_at.naive_utc()],
        )
        .await
        .map_err(|source| ReloadScheduleError::Database(backend(source)))?;
    let value: i64 = row
        .try_get(0)
        .map_err(|source| ReloadScheduleError::Database(backend(source)))?;
    evictor.evict_param(RELOAD_CONFIG_PARAM_NAME).await;
    let Ok(revision) = u32::try_from(value) else {
        return Err(ReloadScheduleError::Revision { value });
    };
    Ok(revision)
}
