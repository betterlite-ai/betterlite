use std::time::Duration;

use betterlite_db::postgres_store::session::PostgresStore;
use tokio::time::sleep;

use super::AgentRegistry;
use super::grant_migration;

mod tests;

const GRANT_MIGRATION_ATTEMPTS: usize = 3;
const GRANT_MIGRATION_PAUSE: Duration = Duration::from_secs(5);

pub(super) async fn run_grant_id_migration(registry: &AgentRegistry, store: &PostgresStore) {
    for attempt in 1..=GRANT_MIGRATION_ATTEMPTS {
        match grant_migration::migrate_legacy_grant_ids(registry, store).await {
            Ok(result) => {
                let rewritten = result.rewritten;
                if rewritten > 0 {
                    tracing::info!(
                        "Rewrote {rewritten} object_permission rows from legacy config agent ids"
                    );
                }
                if result.missed == 0 {
                    return;
                }
                let missed = result.missed;
                tracing::warn!(
                    "Legacy agent grant id migration attempt {attempt}/3 left {missed} rows unmigrated"
                );
            }
            Err(error) => {
                tracing::warn!(
                    "Legacy agent grant id migration attempt {attempt}/3 failed: {error}"
                );
            }
        }
        if attempt < GRANT_MIGRATION_ATTEMPTS {
            sleep(GRANT_MIGRATION_PAUSE).await;
        }
    }
}
