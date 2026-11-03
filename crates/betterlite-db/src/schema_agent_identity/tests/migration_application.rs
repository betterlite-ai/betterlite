use crate::schema_agent_identity::MIGRATION_CATALOG;
use crate::schema_agent_identity::fixtures::PINNED_MIGRATION_CATALOG;
use crate::schema_agent_identity::migrations::{
    ADD_AGENT_KILL_SWITCH, ADD_DAILY_MODEL_USAGE, ADD_MCP_PINNED_TOOLS, AGENT_ENGINE,
    AGENT_IDENTITY, LENS_RUN_HISTORY, RENAME_LENS,
};
use crate::schema_baseline::{BASELINE_MIGRATION, MigrationFile};
use crate::schema_budget_windows::MIGRATION_CATALOG as BUDGET_WINDOWS_CATALOG;
use crate::schema_daily_usage::MIGRATION_CATALOG as DAILY_USAGE_CATALOG;
use crate::schema_identity_history::MIGRATION_CATALOG as IDENTITY_HISTORY_CATALOG;
use crate::schema_managed_resources::MIGRATION_CATALOG as MANAGED_RESOURCES_CATALOG;
use crate::schema_policy_resources::MIGRATION_CATALOG as POLICY_RESOURCES_CATALOG;
use crate::schema_routing_state::MIGRATION_CATALOG as ROUTING_STATE_CATALOG;
use std::str;

async fn apply_baseline(client: &Client) {
    batch_execute(client, str::from_utf8(BASELINE_MIGRATION.bytes).unwrap()).await;
    for catalog in [
        DAILY_USAGE_CATALOG,
        MANAGED_RESOURCES_CATALOG,
        IDENTITY_HISTORY_CATALOG,
        POLICY_RESOURCES_CATALOG,
        ROUTING_STATE_CATALOG,
        BUDGET_WINDOWS_CATALOG,
    ] {
        for file in catalog.files {
            batch_execute(client, str::from_utf8(file.bytes).unwrap()).await;
        }
    }
}
async fn apply_migrations_through(client: &Client, directory: &str) {
    apply_baseline(client).await;
    for file in MIGRATION_CATALOG.files {
        batch_execute(client, str::from_utf8(file.bytes).unwrap()).await;
        if file.directory == directory {
            return;
        }
    }
    panic!("no migration named {directory} in MIGRATION_CATALOG");
}
async fn apply_migrations_all(client: &Client) {
    let last = MIGRATION_CATALOG.files.last().unwrap();
    apply_migrations_through(client, last.directory).await;
}
async fn apply_pinned_migrations_all(client: &Client) {
    apply_baseline(client).await;
    for file in PINNED_MIGRATION_CATALOG.files {
        batch_execute(client, str::from_utf8(file.bytes).unwrap()).await;
    }
}
fn migration_sql_of(directory: &str) -> &'static str {
    let file = MIGRATION_CATALOG
        .files
        .iter()
        .find(|file| file.directory == directory)
        .unwrap_or_else(|| panic!("no migration named {directory} in MIGRATION_CATALOG"));
    str::from_utf8(file.bytes).unwrap()
}
