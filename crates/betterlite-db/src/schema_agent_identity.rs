pub mod fixtures;
mod migrations;
pub const MIGRATION_CATALOG: crate::schema_baseline::MigrationCatalog =
    crate::schema_baseline::MigrationCatalog {
        files: &[
            migrations::AGENT_IDENTITY,
            migrations::ADD_AGENT_KILL_SWITCH,
            migrations::ADD_MCP_PINNED_TOOLS,
            migrations::ADD_DAILY_MODEL_USAGE,
            migrations::AGENT_ENGINE,
            migrations::LENS_RUN_HISTORY,
            migrations::RENAME_LENS,
        ],
    };
mod tests;
