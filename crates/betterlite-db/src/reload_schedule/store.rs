use std::future::Future;
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

// the repository owns its session, so a timed-out call reports the deadline without closing the connection
async fn bounded_table_call<T, F>(
    operation: &'static str,
    deadlines: ScheduleDeadlines,
    call: F,
) -> Result<T, ReloadScheduleError>
where
    F: Future<Output = Result<T, TableError>>,
{
    match timeout(deadlines.statement, call).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(source)) => Err(ReloadScheduleError::Table(source)),
        Err(_elapsed) => Err(ReloadScheduleError::Timeout {
            operation,
            deadline: deadlines.statement,
        }),
    }
}

pub async fn read_reload_schedule(
    client: Option<PrismaClient<'_>>,
    deadlines: ScheduleDeadlines,
) -> Result<Option<ReloadSchedule>, ReloadScheduleError> {
    let repository = ConfigRepository::new(client, false);
    let rows = bounded_table_call(
        "read_reload_schedule",
        deadlines,
        repository
            .table()?
            .as_ref()
            .find::<ReloadConfigRow>(KeyFilter::Equal(RELOAD_CONFIG_PARAM_NAME)),
    )
    .await?;
    rows.first()
        .map(parse_reload_schedule)
        .transpose()
        .map_err(ReloadScheduleError::Parse)
}

// LiteLLM's two admin writes differ in whether a missing row is created, so each runs its own action
pub async fn write_reload_interval(
    client: Option<PrismaClient<'_>>,
    evictor: &dyn ConfigParamEvictor,
    interval_hours: u32,
    deadlines: ScheduleDeadlines,
) -> Result<(), ReloadScheduleError> {
    let config = IntervalConfig {
        interval_hours: Some(interval_hours),
    };
    let param_value = safe_dumps(&config).map_err(ReloadScheduleError::Encode)?;
    let create = ConfigWrite {
        param_name: RELOAD_CONFIG_PARAM_NAME.to_owned(),
        param_value: JsonData::String(param_value),
    };
    let repository = ConfigRepository::new(client, true);
    let table = repository.table()?;
    let action = table
        .as_ref()
        .upsert(RELOAD_CONFIG_PARAM_NAME.to_owned(), create);
    bounded_table_call("write_reload_interval", deadlines, action).await?;
    evictor.evict_param(RELOAD_CONFIG_PARAM_NAME).await?;
    Ok(())
}

pub async fn clear_reload_interval(
    client: Option<PrismaClient<'_>>,
    evictor: &dyn ConfigParamEvictor,
    deadlines: ScheduleDeadlines,
) -> Result<(), ReloadScheduleError> {
    let cleared = safe_dumps(&IntervalConfig {
        interval_hours: None,
    })
    .map_err(ReloadScheduleError::Encode)?;
    let repository = ConfigRepository::new(client, true);
    bounded_table_call(
        "clear_reload_interval",
        deadlines,
        repository.table()?.as_ref().update_many(
            ConfigUpdate {
                param_value: Some(JsonData::String(cleared)),
            },
            RELOAD_CONFIG_PARAM_NAME.to_owned(),
        ),
    )
    .await?;
    evictor.evict_param(RELOAD_CONFIG_PARAM_NAME).await?;
    Ok(())
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
