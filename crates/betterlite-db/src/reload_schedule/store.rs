use std::time::Duration;

use betterlite_redis::config_sync::table_actions::TableActions as _;
use chrono::{DateTime, Utc};
use deadpool_postgres::{Client, GenericClient as _};
use tokio::time::timeout;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use crate::auxiliary_repository::repository::{PrismaClient, TableError};
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::PostgresStore;
use crate::resource_repository::config::{ConfigRepository, ConfigUpdate, ConfigWrite};
use crate::resource_repository::table::KeyFilter;
use crate::search_tools::create::dump::safe_dumps;
use crate::table_ops::{DEFAULT_ACQUIRE_TIMEOUT, DEFAULT_QUERY_TIMEOUT};

use super::RELOAD_CONFIG_PARAM_NAME;
use super::ReloadSchedule;
use super::evictor::{ConfigParamEvictor, EvictError};
use super::parse_schedule::{
    IntervalConfig, ParseScheduleError, ReloadConfigRow, parse_reload_schedule,
};

// each recorded write names its statement, so a Timeout error says which one overran
struct ReloadStatement {
    operation: &'static str,
    sql: &'static str,
}

// update_many without an upsert: a row deleted mid-poll stays deleted and the update affects nothing
const RECORD_RUN: ReloadStatement = ReloadStatement {
    operation: "record_reload_run",
    sql: r#"UPDATE "LiteLLM_Config" SET "last_run_at" = $2 WHERE "param_name" = $1"#,
};

// the create starts the counter at 1; the update increments the stored one, which Postgres serializes per row
const RECORD_MANUAL_RELOAD: ReloadStatement = ReloadStatement {
    operation: "record_manual_reload",
    sql: r#"INSERT INTO "LiteLLM_Config" ("param_name", "last_run_at", "reload_revision")
VALUES ($1, $2, 1)
ON CONFLICT ("param_name") DO UPDATE
SET "last_run_at" = EXCLUDED."last_run_at",
    "reload_revision" = "LiteLLM_Config"."reload_revision" + 1
RETURNING "reload_revision""#,
};

// LiteLLM's awaits carry no deadline of their own; a drained pool or a lock wait fails the call instead of hanging
#[derive(Debug, Clone, Copy)]
pub struct ScheduleDeadlines {
    pub acquire: Duration,
    pub statement: Duration,
}

impl Default for ScheduleDeadlines {
    fn default() -> Self {
        Self {
            acquire: DEFAULT_ACQUIRE_TIMEOUT,
            statement: DEFAULT_QUERY_TIMEOUT,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReloadScheduleError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Table(#[from] TableError),
    #[error(transparent)]
    Parse(#[from] ParseScheduleError),
    #[error(transparent)]
    Evict(#[from] EvictError),
    #[error("the reload schedule value cannot be serialized")]
    Encode(#[source] serde_json::Error),
    #[error("the {operation} did not finish within {deadline:?}")]
    Timeout {
        operation: &'static str,
        deadline: Duration,
    },
    #[error("the published reload revision {value} does not fit a u32")]
    Revision { value: i64 },
}

async fn bounded_session(
    store: &PostgresStore,
    deadlines: ScheduleDeadlines,
) -> Result<Client, ReloadScheduleError> {
    match timeout(deadlines.acquire, store.session()).await {
        Ok(Ok(session)) => Ok(session),
        Ok(Err(source)) => Err(ReloadScheduleError::Database(source)),
        Err(_elapsed) => Err(ReloadScheduleError::Timeout {
            operation: "acquire",
            deadline: deadlines.acquire,
        }),
    }
}

// a statement that overruns its deadline closes its connection, so the server drops the lock wait
async fn bounded_query(
    session: Client,
    operation: &'static str,
    sql: &str,
    parameters: &[&(dyn ToSql + Sync)],
    deadlines: ScheduleDeadlines,
) -> Result<Vec<Row>, ReloadScheduleError> {
    let outcome = timeout(deadlines.statement, session.query(sql, parameters)).await;
    match outcome {
        Ok(Ok(rows)) => Ok(rows),
        Ok(Err(source)) => Err(ReloadScheduleError::Database(backend(source))),
        Err(_elapsed) => {
            drop(Client::take(session));
            Err(ReloadScheduleError::Timeout {
                operation,
                deadline: deadlines.statement,
            })
        }
    }
}

pub async fn read_reload_schedule(
    client: Option<PrismaClient<'_>>,
    deadlines: ScheduleDeadlines,
) -> Result<Option<ReloadSchedule>, ReloadScheduleError> {
    let repository = ConfigRepository::new(client, false);
    let rows = timeout(
        deadlines.statement,
        repository
            .table()?
            .as_ref()
            .find::<ReloadConfigRow>(KeyFilter::Equal(RELOAD_CONFIG_PARAM_NAME)),
    )
    .await
    .map_err(|_elapsed| ReloadScheduleError::Timeout {
        operation: "read_reload_schedule",
        deadline: deadlines.statement,
    })??;
    rows.first()
        .map(parse_reload_schedule)
        .transpose()
        .map_err(ReloadScheduleError::Parse)
}

async fn evict_reload_config(evictor: &dyn ConfigParamEvictor) -> Result<(), ReloadScheduleError> {
    evictor.evict_param(RELOAD_CONFIG_PARAM_NAME).await?;
    Ok(())
}

// LiteLLM's two admin writes differ in whether a missing row is created
#[derive(Clone, Copy)]
enum IntervalWrite {
    Create(u32),
    Clear,
}

// the caller evicts after this returns, so a failed write keeps the cached schedule
async fn store_interval(
    client: Option<PrismaClient<'_>>,
    write: IntervalWrite,
    deadlines: ScheduleDeadlines,
) -> Result<(), ReloadScheduleError> {
    let interval_hours = match write {
        IntervalWrite::Create(hours) => Some(hours),
        IntervalWrite::Clear => None,
    };
    let param_value =
        safe_dumps(&IntervalConfig { interval_hours }).map_err(ReloadScheduleError::Encode)?;
    let repository = ConfigRepository::new(client, true);
    match write {
        IntervalWrite::Create(_) => {
            timeout(
                deadlines.statement,
                repository.table()?.upsert(
                    RELOAD_CONFIG_PARAM_NAME.to_owned(),
                    ConfigWrite {
                        param_name: RELOAD_CONFIG_PARAM_NAME.to_owned(),
                        param_value: JsonData::String(param_value),
                    },
                ),
            )
            .await
            .map_err(|_elapsed| ReloadScheduleError::Timeout {
                operation: "write_reload_interval",
                deadline: deadlines.statement,
            })??;
        }
        IntervalWrite::Clear => {
            timeout(
                deadlines.statement,
                repository.table()?.update_many(
                    ConfigUpdate {
                        param_value: Some(JsonData::String(param_value)),
                    },
                    RELOAD_CONFIG_PARAM_NAME.to_owned(),
                ),
            )
            .await
            .map_err(|_elapsed| ReloadScheduleError::Timeout {
                operation: "clear_reload_interval",
                deadline: deadlines.statement,
            })??;
        }
    }
    Ok(())
}

pub async fn write_reload_interval(
    client: Option<PrismaClient<'_>>,
    evictor: &dyn ConfigParamEvictor,
    interval_hours: u32,
    deadlines: ScheduleDeadlines,
) -> Result<(), ReloadScheduleError> {
    store_interval(client, IntervalWrite::Create(interval_hours), deadlines).await?;
    evict_reload_config(evictor).await
}

pub async fn clear_reload_interval(
    client: Option<PrismaClient<'_>>,
    evictor: &dyn ConfigParamEvictor,
    deadlines: ScheduleDeadlines,
) -> Result<(), ReloadScheduleError> {
    store_interval(client, IntervalWrite::Clear, deadlines).await?;
    evict_reload_config(evictor).await
}

async fn record_statement(
    client: Option<PrismaClient<'_>>,
    evictor: &dyn ConfigParamEvictor,
    statement: &ReloadStatement,
    ran_at: DateTime<Utc>,
    deadlines: ScheduleDeadlines,
) -> Result<Vec<Row>, ReloadScheduleError> {
    let store = ConfigRepository::new(client, true).prisma_client()?.writer;
    let session = bounded_session(store, deadlines).await?;
    let rows = bounded_query(
        session,
        statement.operation,
        statement.sql,
        &[&RELOAD_CONFIG_PARAM_NAME, &ran_at.naive_utc()],
        deadlines,
    )
    .await?;
    evictor.evict_param(RELOAD_CONFIG_PARAM_NAME).await?;
    Ok(rows)
}

pub async fn record_reload_run(
    client: Option<PrismaClient<'_>>,
    evictor: &dyn ConfigParamEvictor,
    ran_at: DateTime<Utc>,
    deadlines: ScheduleDeadlines,
) -> Result<(), ReloadScheduleError> {
    record_statement(client, evictor, &RECORD_RUN, ran_at, deadlines).await?;
    Ok(())
}

pub async fn record_manual_reload(
    client: Option<PrismaClient<'_>>,
    evictor: &dyn ConfigParamEvictor,
    ran_at: DateTime<Utc>,
    deadlines: ScheduleDeadlines,
) -> Result<u32, ReloadScheduleError> {
    let rows = record_statement(client, evictor, &RECORD_MANUAL_RELOAD, ran_at, deadlines).await?;
    let Some(row) = rows.into_iter().next() else {
        return Err(ReloadScheduleError::Database(DbError::CreateNull));
    };
    let value: i64 = row
        .try_get(0)
        .map_err(|source| ReloadScheduleError::Database(backend(source)))?;
    u32::try_from(value).map_err(|_out_of_range| ReloadScheduleError::Revision { value })
}
