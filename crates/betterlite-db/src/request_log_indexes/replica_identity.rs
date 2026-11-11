use std::time::Duration;

use secrecy::SecretString;
use tokio::time::timeout;

use super::connect;
use super::database::IndexDatabase;

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

pub async fn apply_replica_identity_full(
    database_url: &SecretString,
    command_timeout: Duration,
) -> bool {
    match connect::connect(database_url, command_timeout).await {
        Ok(database) => apply_on(&database, command_timeout).await,
        Err(error) => {
            tracing::info!("{APPLY_MESSAGE}");
            tracing::error!(
                "Could not run the REPLICA IDENTITY FULL statement. Logical replication consumers may reject updates to these tables. Error: {error}"
            );
            false
        }
    }
}

pub(crate) async fn apply_on<D>(database: &D, command_timeout: Duration) -> bool
where
    D: IndexDatabase,
{
    tracing::info!("{APPLY_MESSAGE}");
    let executed = timeout(command_timeout, database.execute(REPLICA_IDENTITY_FULL_SQL)).await;
    match executed {
        Err(_) => {
            tracing::error!("Timed out setting REPLICA IDENTITY FULL on LiteLLM tables");
            false
        }
        Ok(Err(error)) => {
            tracing::error!(
                "Failed to set REPLICA IDENTITY FULL. Logical replication consumers may reject updates to these tables. Grant table ownership to the migration user, or apply `ALTER TABLE ... REPLICA IDENTITY FULL` by hand. Error: {error}"
            );
            false
        }
        Ok(Ok(())) => {
            tracing::info!("REPLICA IDENTITY FULL applied to LiteLLM tables");
            true
        }
    }
}
