use crate::schema_baseline::MigrationCatalog;

pub mod fixtures;
mod migrations;

pub const MIGRATION_CATALOG: MigrationCatalog = MigrationCatalog {
    files: &[
        migrations::ADD_ADAPTIVE_ROUTER_TABLES,
        migrations::ADD_MEMORY_TABLE,
        migrations::ADD_TEAM_TOTAL_SPEND,
        migrations::SEARCH_TOOLS_OBJECT_PERM,
        migrations::WORKFLOW_RUNS_TABLES,
        migrations::MANAGED_RESOURCE_TEAM_OWNER,
        migrations::ADD_MCP_DELEGATE_AUTH,
        migrations::ADD_PROXY_MODEL_BLOCKED,
        migrations::ADD_MCP_ENV_VARS,
        migrations::ADD_MCP_OAUTH_PASSTHROUGH,
        migrations::ADD_OAUTH2_FLOW_TO_MCP_SERVERS,
        migrations::ADD_MCP_SERVER_TIMEOUT,
        migrations::ADD_MCP_TOOL_SEARCH_ENABLED,
        migrations::ADD_MCP_MAX_CONCURRENCY,
        migrations::ADD_MCP_TOKEN_EXCHANGE,
        migrations::ADD_TOKEN_BUDGET_FALLBACKS,
        migrations::ADD_MCP_EXCHANGE_PROFILE,
        migrations::ADD_MCP_DCR_BRIDGE,
        migrations::ADD_DAILY_TEAM_PTU,
        migrations::ADD_TOKEN_KEY_TYPE,
        migrations::ADD_ISSUER_TO_MCP_SERVER_TABLE,
        migrations::ADD_COMPRESSION_SAVED_TOKENS,
        migrations::ADD_MCP_OAUTH_CLIENT,
        migrations::JWT_KEY_CASCADE_DELETE,
        migrations::ADD_SAVINGS_SPEND,
        migrations::ADD_SSO_IDENTITY_ASSERTION,
        migrations::ADD_SPEND_LOG_TOOL_INDEX,
        migrations::ADD_DAILY_TOOL_SPEND,
        migrations::ADD_CONFIG_RELOAD_TRACKING,
        migrations::ADD_MANAGED_OBJECT_TAGS,
        migrations::ADD_AUTOROUTER_SAVINGS_SPEND,
        migrations::ADD_DAILY_GATEWAY_REQUESTS,
        migrations::ADD_BUDGET_WINDOW_SPEND_TABLE,
        migrations::ADD_AUTOROUTER_SESSION_ROLLUP,
        migrations::ADD_AUTOROUTER_TIER_TURNS,
        migrations::ADD_TOKEN_SETTINGS_UPDATED,
    ],
};

mod tests;
