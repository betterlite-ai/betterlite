use crate::schema_baseline::{MigrationCatalog, MigrationFile};

pub mod fixtures;

pub const ADD_ADAPTIVE_ROUTER_TABLES: MigrationFile = MigrationFile {
    directory: "20260418000000_add_adaptive_router_tables",
    bytes: include_bytes!("../migrations/20260418000000_add_adaptive_router_tables/migration.sql"),
    sha256: "b49bdecc8510bbe30eb3810761cbde182a37575cd012e38fff60829a3c32e8a7",
};

pub const ADD_MEMORY_TABLE: MigrationFile = MigrationFile {
    directory: "20260421120000_add_memory_table",
    bytes: include_bytes!("../migrations/20260421120000_add_memory_table/migration.sql"),
    sha256: "a39a409b748dc9c65ab7f182cc668d21c89a8489f1859899056fab60a7c8ba1e",
};

pub const ADD_TEAM_TOTAL_SPEND: MigrationFile = MigrationFile {
    directory: "20260421135425_add_team_membership_total_spend",
    bytes: include_bytes!(
        "../migrations/20260421135425_add_team_membership_total_spend/migration.sql"
    ),
    sha256: "f955bbe1ba1812b4023f0e9bcaacb274006d39782a403e56620051f955b22303",
};

pub const SEARCH_TOOLS_OBJECT_PERM: MigrationFile = MigrationFile {
    directory: "20260429120000_search_tools_on_object_permission",
    bytes: include_bytes!(
        "../migrations/20260429120000_search_tools_on_object_permission/migration.sql"
    ),
    sha256: "dab418bc243b4a928ca2fca2a49621900c1d2223fde0aa5c9c6027ca2d8b5558",
};

pub const WORKFLOW_RUNS_TABLES: MigrationFile = MigrationFile {
    directory: "20260429161855_workflow_runs_tables",
    bytes: include_bytes!("../migrations/20260429161855_workflow_runs_tables/migration.sql"),
    sha256: "ef37c5ce9e82708631bf7692b2bed8eb9efec1b63812b80141e69dffbf3958e9",
};

pub const MANAGED_RESOURCE_TEAM_OWNER: MigrationFile = MigrationFile {
    directory: "20260501195714_managed_resource_team_owner",
    bytes: include_bytes!("../migrations/20260501195714_managed_resource_team_owner/migration.sql"),
    sha256: "70a413cf659a8263cc4cc3d90942fa13c11569f6a29dac63aabfe53791ea7f57",
};

pub const ADD_MCP_DELEGATE_AUTH: MigrationFile = MigrationFile {
    directory: "20260513120000_add_delegate_auth_to_upstream_to_mcp_servers",
    bytes: include_bytes!(
        "../migrations/20260513120000_add_delegate_auth_to_upstream_to_mcp_servers/migration.sql"
    ),
    sha256: "8edde816aef67759bbdb455276a3aa02e15e2e96892d8623cdb58092d393992e",
};

pub const ADD_PROXY_MODEL_BLOCKED: MigrationFile = MigrationFile {
    directory: "20260514120000_add_blocked_to_proxy_model_table",
    bytes: include_bytes!(
        "../migrations/20260514120000_add_blocked_to_proxy_model_table/migration.sql"
    ),
    sha256: "6079a465b829311a62c9fccc9c15447754841e683b3862551819d02cc59868fc",
};

pub const ADD_MCP_ENV_VARS: MigrationFile = MigrationFile {
    directory: "20260520120000_add_mcp_env_vars",
    bytes: include_bytes!("../migrations/20260520120000_add_mcp_env_vars/migration.sql"),
    sha256: "b1884c51b26eaf06412782635b88f7073efbf8a93c4a6b647eb1f504e53e7bc6",
};

pub const ADD_MCP_OAUTH_PASSTHROUGH: MigrationFile = MigrationFile {
    directory: "20260526120000_add_oauth_passthrough_to_mcp_servers",
    bytes: include_bytes!(
        "../migrations/20260526120000_add_oauth_passthrough_to_mcp_servers/migration.sql"
    ),
    sha256: "12fc51a97ad28eb8c2f69841ee25d30f3f394b73f689d99952adb5a8db2de224",
};

pub const ADD_OAUTH2_FLOW_TO_MCP_SERVERS: MigrationFile = MigrationFile {
    directory: "20260604120000_add_oauth2_flow_to_mcp_servers",
    bytes: include_bytes!(
        "../migrations/20260604120000_add_oauth2_flow_to_mcp_servers/migration.sql"
    ),
    sha256: "37a30575e319fe6d071ae0169f920822af0cd1dc98bce17a60afa21548dd6aa0",
};

pub const ADD_MCP_SERVER_TIMEOUT: MigrationFile = MigrationFile {
    directory: "20260605182307_add_timeout_to_mcp_server_table",
    bytes: include_bytes!(
        "../migrations/20260605182307_add_timeout_to_mcp_server_table/migration.sql"
    ),
    sha256: "6b8dd9fbce6d69844943ae1ad2735bdcc9b4bca8947a4478446118ed813537fa",
};

pub const ADD_MCP_TOOL_SEARCH_ENABLED: MigrationFile = MigrationFile {
    directory: "20260626120000_add_mcp_tool_search_enabled",
    bytes: include_bytes!("../migrations/20260626120000_add_mcp_tool_search_enabled/migration.sql"),
    sha256: "58ba614cb29ed40b724f02a5657b1eb04294c528f55c889413fa1b4f8e1a0a2b",
};

pub const ADD_MCP_MAX_CONCURRENCY: MigrationFile = MigrationFile {
    directory: "20260629000000_add_max_concurrent_requests_to_mcp_server_table",
    bytes: include_bytes!(
        "../migrations/20260629000000_add_max_concurrent_requests_to_mcp_server_table/migration.sql"
    ),
    sha256: "ab8e6e84af7b3080d559c5e2023752b7f1bd69c94111284fe1d878bcdf4458ea",
};

pub const ADD_MCP_TOKEN_EXCHANGE: MigrationFile = MigrationFile {
    directory: "20260630120000_add_token_exchange_to_mcp_servers",
    bytes: include_bytes!(
        "../migrations/20260630120000_add_token_exchange_to_mcp_servers/migration.sql"
    ),
    sha256: "ddc9a89115f25ec043b6dfd46eff43e37289db55ab3e82a74290660d308a0399",
};

pub const ADD_TOKEN_BUDGET_FALLBACKS: MigrationFile = MigrationFile {
    directory: "20260630190000_add_budget_fallbacks_to_litellm_verification_token",
    bytes: include_bytes!(
        "../migrations/20260630190000_add_budget_fallbacks_to_litellm_verification_token/migration.sql"
    ),
    sha256: "e505c9f94735b209b790293b0a02e14a9b7fd306d144c0d6138190874105e02e",
};

pub const ADD_MCP_EXCHANGE_PROFILE: MigrationFile = MigrationFile {
    directory: "20260703120000_add_token_exchange_profile_to_mcp_servers",
    bytes: include_bytes!(
        "../migrations/20260703120000_add_token_exchange_profile_to_mcp_servers/migration.sql"
    ),
    sha256: "c17998f21e0122401c7f3e4d11f02b7c3924da6ae9a0f5a17bb4920d7b5b3ccc",
};

pub const ADD_MCP_DCR_BRIDGE: MigrationFile = MigrationFile {
    directory: "20260710000000_add_dcr_bridge_to_mcp_server_table",
    bytes: include_bytes!(
        "../migrations/20260710000000_add_dcr_bridge_to_mcp_server_table/migration.sql"
    ),
    sha256: "8071e33b4af00586d53adfecfac43a6ce3ece46aadabc306e43083c8c429faa4",
};

pub const ADD_DAILY_TEAM_PTU: MigrationFile = MigrationFile {
    directory: "20260713010000_add_ptu_columns_to_daily_team_spend",
    bytes: include_bytes!(
        "../migrations/20260713010000_add_ptu_columns_to_daily_team_spend/migration.sql"
    ),
    sha256: "b8b667c881330111a4f034e20d489b962b2c7599ca8ebbd54cadfcb9e7446076",
};

pub const ADD_TOKEN_KEY_TYPE: MigrationFile = MigrationFile {
    directory: "20260713230852_add_key_type_to_litellm_verification_token",
    bytes: include_bytes!(
        "../migrations/20260713230852_add_key_type_to_litellm_verification_token/migration.sql"
    ),
    sha256: "845c8a7b0a360fa1aad7420c16b853c526e53b77f34da2330088bc326570ef3e",
};

pub const ADD_ISSUER_TO_MCP_SERVER_TABLE: MigrationFile = MigrationFile {
    directory: "20260715000000_add_issuer_to_mcp_server_table",
    bytes: include_bytes!(
        "../migrations/20260715000000_add_issuer_to_mcp_server_table/migration.sql"
    ),
    sha256: "bb5a30704d0ff8a32635b757a34027d84170ec0ad114bcaf1ca71e771f57b19a",
};

pub const ADD_COMPRESSION_SAVED_TOKENS: MigrationFile = MigrationFile {
    directory: "20260717000000_add_compression_saved_tokens",
    bytes: include_bytes!(
        "../migrations/20260717000000_add_compression_saved_tokens/migration.sql"
    ),
    sha256: "3bd9818383657f7399aa7e5b5acb8743f3d684063cad4e0f962927f8574eefe0",
};

pub const ADD_MCP_OAUTH_CLIENT: MigrationFile = MigrationFile {
    directory: "20260717000000_add_mcp_server_oauth_client_table",
    bytes: include_bytes!(
        "../migrations/20260717000000_add_mcp_server_oauth_client_table/migration.sql"
    ),
    sha256: "9d9b8d465b5c7974d73798f07b74496a5ff5a615e9fc646c8eac3651f50eb94b",
};

pub const JWT_KEY_CASCADE_DELETE: MigrationFile = MigrationFile {
    directory: "20260717000000_cascade_delete_jwt_key_mapping_on_token_delete",
    bytes: include_bytes!(
        "../migrations/20260717000000_cascade_delete_jwt_key_mapping_on_token_delete/migration.sql"
    ),
    sha256: "a9d57b2cfb9e268734ed4d43786f3423f333c51d8f6f698ec4a3b1ffb77c446a",
};

pub const ADD_SAVINGS_SPEND: MigrationFile = MigrationFile {
    directory: "20260718000000_add_savings_spend",
    bytes: include_bytes!("../migrations/20260718000000_add_savings_spend/migration.sql"),
    sha256: "09d40b5beacaf2c2bf1c73da2fe0e67e2812e3fd6ee1977c47ae9417eda0667d",
};

pub const ADD_SSO_IDENTITY_ASSERTION: MigrationFile = MigrationFile {
    directory: "20260721000000_add_sso_identity_assertion",
    bytes: include_bytes!("../migrations/20260721000000_add_sso_identity_assertion/migration.sql"),
    sha256: "61df604554a40f55fd44a1f94d20b154eea3d10bc83a399d64ed42a784a37e71",
};

pub const ADD_SPEND_LOG_TOOL_INDEX: MigrationFile = MigrationFile {
    directory: "20260724000000_add_spend_log_tool_index_start_time_idx",
    bytes: include_bytes!(
        "../migrations/20260724000000_add_spend_log_tool_index_start_time_idx/migration.sql"
    ),
    sha256: "3a73fe2955610df1fd635e2ab4ffb33004db5d0c03221eda799ac45455d7af09",
};

pub const ADD_DAILY_TOOL_SPEND: MigrationFile = MigrationFile {
    directory: "20260725000000_add_daily_tool_spend",
    bytes: include_bytes!("../migrations/20260725000000_add_daily_tool_spend/migration.sql"),
    sha256: "626702edb86035095d6ae9cd21ee218bc7ed98267c2472a9918b1f7057957ab2",
};

pub const ADD_CONFIG_RELOAD_TRACKING: MigrationFile = MigrationFile {
    directory: "20260729000000_add_reload_tracking_to_litellm_config",
    bytes: include_bytes!(
        "../migrations/20260729000000_add_reload_tracking_to_litellm_config/migration.sql"
    ),
    sha256: "c2b92f39fc51fc9a23ebc90eebc648e5cd1eaa70fcc92db7bb43cf1b6d9a2e7e",
};

pub const ADD_MANAGED_OBJECT_TAGS: MigrationFile = MigrationFile {
    directory: "20260730000000_add_api_key_and_request_tags_to_managed_object_table",
    bytes: include_bytes!(
        "../migrations/20260730000000_add_api_key_and_request_tags_to_managed_object_table/migration.sql"
    ),
    sha256: "dbfd2ddf670c8ee0d6016c38db9858334d2593f739e0778ccf60691e645819e1",
};

pub const ADD_AUTOROUTER_SAVINGS_SPEND: MigrationFile = MigrationFile {
    directory: "20260731000000_add_autorouter_savings_spend",
    bytes: include_bytes!(
        "../migrations/20260731000000_add_autorouter_savings_spend/migration.sql"
    ),
    sha256: "7bb2048d3377d9a237a5650f757992aada6bc9d88a003b326d283397541f330e",
};

pub const ADD_DAILY_GATEWAY_REQUESTS: MigrationFile = MigrationFile {
    directory: "20260803000000_add_daily_gateway_requests",
    bytes: include_bytes!("../migrations/20260803000000_add_daily_gateway_requests/migration.sql"),
    sha256: "d052bf6f7fead1abbe9dce9b7adda7a6497acd685321f91b819ecd1ac34eb551",
};

pub const ADD_BUDGET_WINDOW_SPEND_TABLE: MigrationFile = MigrationFile {
    directory: "20260804162853_add_budget_window_spend_table",
    bytes: include_bytes!(
        "../migrations/20260804162853_add_budget_window_spend_table/migration.sql"
    ),
    sha256: "13eb8d2505475ee58eb957c054133fdf570f55e3e0eefb5244a5996c6c8347a6",
};

pub const ADD_AUTOROUTER_SESSION_ROLLUP: MigrationFile = MigrationFile {
    directory: "20260805000000_add_autorouter_session_rollup",
    bytes: include_bytes!(
        "../migrations/20260805000000_add_autorouter_session_rollup/migration.sql"
    ),
    sha256: "25607951a7a70e13670a7209c1b711cf22e758b441028b4ce6d9e76c662d184f",
};

pub const ADD_AUTOROUTER_TIER_TURNS: MigrationFile = MigrationFile {
    directory: "20260807000000_add_autorouter_session_tier_turns",
    bytes: include_bytes!(
        "../migrations/20260807000000_add_autorouter_session_tier_turns/migration.sql"
    ),
    sha256: "22e5b257dcd2d8695f9b76eddc5f6543a245fdcea14bf5ba6f0643eebb988a54",
};

pub const ADD_TOKEN_SETTINGS_UPDATED: MigrationFile = MigrationFile {
    directory: "20260810000000_add_verificationtoken_settings_updated_at",
    bytes: include_bytes!(
        "../migrations/20260810000000_add_verificationtoken_settings_updated_at/migration.sql"
    ),
    sha256: "420c8634a5febc76b118fcb9295b0dfd73579d58396ac85746f4098c46d34739",
};

pub const MIGRATION_CATALOG: MigrationCatalog = MigrationCatalog {
    files: &[
        ADD_ADAPTIVE_ROUTER_TABLES,
        ADD_MEMORY_TABLE,
        ADD_TEAM_TOTAL_SPEND,
        SEARCH_TOOLS_OBJECT_PERM,
        WORKFLOW_RUNS_TABLES,
        MANAGED_RESOURCE_TEAM_OWNER,
        ADD_MCP_DELEGATE_AUTH,
        ADD_PROXY_MODEL_BLOCKED,
        ADD_MCP_ENV_VARS,
        ADD_MCP_OAUTH_PASSTHROUGH,
        ADD_OAUTH2_FLOW_TO_MCP_SERVERS,
        ADD_MCP_SERVER_TIMEOUT,
        ADD_MCP_TOOL_SEARCH_ENABLED,
        ADD_MCP_MAX_CONCURRENCY,
        ADD_MCP_TOKEN_EXCHANGE,
        ADD_TOKEN_BUDGET_FALLBACKS,
        ADD_MCP_EXCHANGE_PROFILE,
        ADD_MCP_DCR_BRIDGE,
        ADD_DAILY_TEAM_PTU,
        ADD_TOKEN_KEY_TYPE,
        ADD_ISSUER_TO_MCP_SERVER_TABLE,
        ADD_COMPRESSION_SAVED_TOKENS,
        ADD_MCP_OAUTH_CLIENT,
        JWT_KEY_CASCADE_DELETE,
        ADD_SAVINGS_SPEND,
        ADD_SSO_IDENTITY_ASSERTION,
        ADD_SPEND_LOG_TOOL_INDEX,
        ADD_DAILY_TOOL_SPEND,
        ADD_CONFIG_RELOAD_TRACKING,
        ADD_MANAGED_OBJECT_TAGS,
        ADD_AUTOROUTER_SAVINGS_SPEND,
        ADD_DAILY_GATEWAY_REQUESTS,
        ADD_BUDGET_WINDOW_SPEND_TABLE,
        ADD_AUTOROUTER_SESSION_ROLLUP,
        ADD_AUTOROUTER_TIER_TURNS,
        ADD_TOKEN_SETTINGS_UPDATED,
    ],
};

mod tests;
