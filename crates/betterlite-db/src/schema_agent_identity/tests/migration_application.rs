use crate::schema_agent_identity::MIGRATION_CATALOG;
use crate::schema_agent_identity::fixtures::PINNED_MIGRATION_CATALOG;
use crate::schema_agent_identity::migrations::{
    ADD_AGENT_KILL_SWITCH, ADD_DAILY_MODEL_USAGE, ADD_MCP_PINNED_TOOLS, AGENT_ENGINE,
    AGENT_IDENTITY, LENS_RUN_HISTORY, RENAME_LENS,
};
use crate::schema_baseline::MigrationFile;
use std::str;
async fn apply_migrations_through(client: &Client, directory: &str) {
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
