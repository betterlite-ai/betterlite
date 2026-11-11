use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::{Instant, timeout};

use crate::database_url_policy::query::url_query_value;

use super::RequestLogIndexError;
use super::connect;
use super::database::{DatabaseFailure, IndexDatabase, PostgresDatabase};

pub(crate) const REPLICA_IDENTITY_FULL_SQL: &str = r"
DO $$
DECLARE
    target regclass;
BEGIN
    SET LOCAL lock_timeout = '5s';
    FOR target IN
        SELECT c.oid::regclass
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE c.relkind = 'r'
          AND c.relreplident <> 'f'
          AND n.nspname = ANY (current_schemas(false))
          AND c.relname LIKE 'LiteLLM\_%'
    LOOP
        BEGIN
            EXECUTE format('ALTER TABLE %s REPLICA IDENTITY FULL', target);
        EXCEPTION WHEN lock_not_available THEN
            RAISE WARNING 'REPLICA IDENTITY FULL skipped for %: table busy, retrying next run', target;
        END;
    END LOOP;
END
$$;
";

const APPLY_MESSAGE: &str = "Applying REPLICA IDENTITY FULL to LiteLLM tables";

// One command budget covers the connection and the DO, like the Prisma subprocess timeout
pub async fn apply_replica_identity_full(
    database_url: &SecretString,
    command_timeout: Duration,
) -> bool {
    let started = Instant::now();
    match timeout(command_timeout, open_with_schema(database_url)).await {
        Ok(Ok(database)) => apply_on(&database, remaining(command_timeout, started)).await,
        Ok(Err(error)) => {
            tracing::info!("{APPLY_MESSAGE}");
            report::<RequestLogIndexError>(Err(ReplicaFailure::Connection(error)))
        }
        Err(_elapsed) => {
            tracing::info!("{APPLY_MESSAGE}");
            report::<RequestLogIndexError>(Err(ReplicaFailure::TimedOut))
        }
    }
}

// Prisma sets search_path from the URL's schema parameter, which the native options drop
async fn open_with_schema(
    database_url: &SecretString,
) -> Result<PostgresDatabase, RequestLogIndexError> {
    let database = connect::open(database_url).await?;
    if let Some(schema) = schema_parameter(database_url) {
        database
            .client
            .query_one("SELECT set_config('search_path', $1, false)", &[&schema])
            .await
            .map_err(RequestLogIndexError::from)?;
    }
    Ok(database)
}

fn schema_parameter(database_url: &SecretString) -> Option<String> {
    let schema = url_query_value(Some(database_url.expose_secret()), "schema").ok()?;
    if schema.is_empty() {
        return None;
    }
    Some(format!("\"{}\"", schema.replace('"', "\"\"")))
}

pub(crate) fn remaining(command_timeout: Duration, started: Instant) -> Duration {
    command_timeout
        .checked_sub(started.elapsed())
        .unwrap_or(Duration::ZERO)
}

pub(crate) async fn apply_on<D>(database: &D, command_timeout: Duration) -> bool
where
    D: IndexDatabase,
{
    tracing::info!("{APPLY_MESSAGE}");
    report(bounded_execute(database, command_timeout).await)
}

async fn bounded_execute<D>(
    database: &D,
    command_timeout: Duration,
) -> Result<(), ReplicaFailure<D::Error>>
where
    D: IndexDatabase,
{
    match timeout(command_timeout, database.execute(REPLICA_IDENTITY_FULL_SQL)).await {
        Err(_elapsed) => Err(ReplicaFailure::TimedOut),
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(ReplicaFailure::Statement(error)),
    }
}

enum ReplicaFailure<E> {
    TimedOut,
    Connection(RequestLogIndexError),
    Statement(E),
}

fn report<E>(outcome: Result<(), ReplicaFailure<E>>) -> bool
where
    E: DatabaseFailure,
{
    match outcome {
        Ok(()) => {
            tracing::info!("REPLICA IDENTITY FULL applied to LiteLLM tables");
            true
        }
        Err(ReplicaFailure::TimedOut) => {
            tracing::error!("Timed out setting REPLICA IDENTITY FULL on LiteLLM tables");
            false
        }
        Err(ReplicaFailure::Connection(error)) => {
            tracing::error!(
                "Could not run the REPLICA IDENTITY FULL statement. Logical replication consumers may reject updates to these tables. Error: {error}"
            );
            false
        }
        Err(ReplicaFailure::Statement(error)) => {
            tracing::error!(
                "Failed to set REPLICA IDENTITY FULL. Logical replication consumers may reject updates to these tables. Grant table ownership to the migration user, or apply `ALTER TABLE ... REPLICA IDENTITY FULL` by hand. Error: {error}"
            );
            false
        }
    }
}
