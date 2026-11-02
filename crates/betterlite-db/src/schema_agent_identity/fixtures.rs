#![cfg(test)]

use crate::schema_baseline::{MigrationCatalog, MigrationFile};

const PINNED_AGENT_IDENTITY: MigrationFile = MigrationFile {
    directory: "20260921190000_agent_identity",
    bytes: include_bytes!("fixtures/20260921190000_agent_identity.sql"),
    sha256: "4a9d6f79f0cfe0eef9b76d1dd4c676b93be64d5621fd9f4c11f5870385991bba",
};

const PINNED_ADD_AGENT_KILL_SWITCH: MigrationFile = MigrationFile {
    directory: "20260923000000_add_agent_kill_switch",
    bytes: include_bytes!("fixtures/20260923000000_add_agent_kill_switch.sql"),
    sha256: "5202b1314da4432a30700c558b6ef3c234a3057376deca81f0dad00f4293838a",
};

const PINNED_ADD_MCP_PINNED_TOOLS: MigrationFile = MigrationFile {
    directory: "20260925000000_add_mcp_pinned_tools",
    bytes: include_bytes!("fixtures/20260925000000_add_mcp_pinned_tools.sql"),
    sha256: "23f89722e760e15fc22cf90caa439d7895cd60c32cc6add6b1ca449993f88562",
};

const PINNED_ADD_DAILY_MODEL_USAGE: MigrationFile = MigrationFile {
    directory: "20260928000000_add_daily_model_usage",
    bytes: include_bytes!("fixtures/20260928000000_add_daily_model_usage.sql"),
    sha256: "2eb2711cabd6f75670849f40ee1d1f7f48d468f22d7871c69dbede02d60cb63d",
};

const PINNED_AGENT_ENGINE: MigrationFile = MigrationFile {
    directory: "20260930000000_agent_engine",
    bytes: include_bytes!("fixtures/20260930000000_agent_engine.sql"),
    sha256: "c08a58b26e04b2a83bb4c58864e0a9608941c3561dc7dbfc9c12604336f392d9",
};

const PINNED_LENS_RUN_HISTORY: MigrationFile = MigrationFile {
    directory: "20261001000000_lens_run_history",
    bytes: include_bytes!("fixtures/20261001000000_lens_run_history.sql"),
    sha256: "200f7b134499282a235b0be261d520286066c5f7fa10bc461fd29c6d749a0b61",
};

const PINNED_RENAME_LENS: MigrationFile = MigrationFile {
    directory: "20261001100000_rename_lens",
    bytes: include_bytes!("fixtures/20261001100000_rename_lens.sql"),
    sha256: "ca2c6b8bda8b16381e9b707d43d914c800923c8b476b4da12d3ad0b4179a3603",
};

pub(super) const PINNED_MIGRATION_CATALOG: MigrationCatalog = MigrationCatalog {
    files: &[
        PINNED_AGENT_IDENTITY,
        PINNED_ADD_AGENT_KILL_SWITCH,
        PINNED_ADD_MCP_PINNED_TOOLS,
        PINNED_ADD_DAILY_MODEL_USAGE,
        PINNED_AGENT_ENGINE,
        PINNED_LENS_RUN_HISTORY,
        PINNED_RENAME_LENS,
    ],
};
