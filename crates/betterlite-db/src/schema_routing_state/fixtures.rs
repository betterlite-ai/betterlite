#![cfg(test)]

use crate::schema_baseline::{MigrationCatalog, MigrationFile};

const PINNED_ADAPTIVE_ROUTER: MigrationFile = MigrationFile {
    directory: "20260418000000_add_adaptive_router_tables",
    bytes: include_bytes!("fixtures/20260418000000_add_adaptive_router_tables.sql"),
    sha256: "b49bdecc8510bbe30eb3810761cbde182a37575cd012e38fff60829a3c32e8a7",
};

const PINNED_ADD_MEMORY_TABLE: MigrationFile = MigrationFile {
    directory: "20260421120000_add_memory_table",
    bytes: include_bytes!("fixtures/20260421120000_add_memory_table.sql"),
    sha256: "a39a409b748dc9c65ab7f182cc668d21c89a8489f1859899056fab60a7c8ba1e",
};

const PINNED_TEAM_TOTAL_SPEND: MigrationFile = MigrationFile {
    directory: "20260421135425_add_team_membership_total_spend",
    bytes: include_bytes!("fixtures/20260421135425_add_team_membership_total_spend.sql"),
    sha256: "686be79c5c68c056c7473f27374767ae6c952fe09e95ea7b8dd13047a09fad8b",
};

const PINNED_TOOLS_OBJ_PERM: MigrationFile = MigrationFile {
    directory: "20260429120000_search_tools_on_object_permission",
    bytes: include_bytes!("fixtures/20260429120000_search_tools_on_object_permission.sql"),
    sha256: "dab418bc243b4a928ca2fca2a49621900c1d2223fde0aa5c9c6027ca2d8b5558",
};

const PINNED_WORKFLOW_RUNS_TABLES: MigrationFile = MigrationFile {
    directory: "20260429161855_workflow_runs_tables",
    bytes: include_bytes!("fixtures/20260429161855_workflow_runs_tables.sql"),
    sha256: "6293c4f78ea52d9e64503a0b284e9da960dd6351ffe2210b74ec5494607c6f67",
};

const PINNED_RESOURCE_OWNER: MigrationFile = MigrationFile {
    directory: "20260501195714_managed_resource_team_owner",
    bytes: include_bytes!("fixtures/20260501195714_managed_resource_team_owner.sql"),
    sha256: "70a413cf659a8263cc4cc3d90942fa13c11569f6a29dac63aabfe53791ea7f57",
};

const PINNED_DELEGATE_AUTH: MigrationFile = MigrationFile {
    directory: "20260513120000_add_delegate_auth_to_upstream_to_mcp_servers",
    bytes: include_bytes!(
        "fixtures/20260513120000_add_delegate_auth_to_upstream_to_mcp_servers.sql"
    ),
    sha256: "8edde816aef67759bbdb455276a3aa02e15e2e96892d8623cdb58092d393992e",
};

const PINNED_PROXY_BLOCKED: MigrationFile = MigrationFile {
    directory: "20260514120000_add_blocked_to_proxy_model_table",
    bytes: include_bytes!("fixtures/20260514120000_add_blocked_to_proxy_model_table.sql"),
    sha256: "6079a465b829311a62c9fccc9c15447754841e683b3862551819d02cc59868fc",
};

const PINNED_ADD_MCP_ENV_VARS: MigrationFile = MigrationFile {
    directory: "20260520120000_add_mcp_env_vars",
    bytes: include_bytes!("fixtures/20260520120000_add_mcp_env_vars.sql"),
    sha256: "b1884c51b26eaf06412782635b88f7073efbf8a93c4a6b647eb1f504e53e7bc6",
};

const PINNED_OAUTH_PASSTHRU: MigrationFile = MigrationFile {
    directory: "20260526120000_add_oauth_passthrough_to_mcp_servers",
    bytes: include_bytes!("fixtures/20260526120000_add_oauth_passthrough_to_mcp_servers.sql"),
    sha256: "12fc51a97ad28eb8c2f69841ee25d30f3f394b73f689d99952adb5a8db2de224",
};

const PINNED_OAUTH2_FLOW: MigrationFile = MigrationFile {
    directory: "20260604120000_add_oauth2_flow_to_mcp_servers",
    bytes: include_bytes!("fixtures/20260604120000_add_oauth2_flow_to_mcp_servers.sql"),
    sha256: "37a30575e319fe6d071ae0169f920822af0cd1dc98bce17a60afa21548dd6aa0",
};

const PINNED_MCP_TIMEOUT: MigrationFile = MigrationFile {
    directory: "20260605182307_add_timeout_to_mcp_server_table",
    bytes: include_bytes!("fixtures/20260605182307_add_timeout_to_mcp_server_table.sql"),
    sha256: "04ad0398ccf6dc0c03a1f1520f4461b7fe34347654f81d19a2a3bef8e8ef2e89",
};

const PINNED_MCP_TOOL_SEARCH: MigrationFile = MigrationFile {
    directory: "20260626120000_add_mcp_tool_search_enabled",
    bytes: include_bytes!("fixtures/20260626120000_add_mcp_tool_search_enabled.sql"),
    sha256: "58ba614cb29ed40b724f02a5657b1eb04294c528f55c889413fa1b4f8e1a0a2b",
};

const PINNED_MCP_CONCURRENCY: MigrationFile = MigrationFile {
    directory: "20260629000000_add_max_concurrent_requests_to_mcp_server_table",
    bytes: include_bytes!(
        "fixtures/20260629000000_add_max_concurrent_requests_to_mcp_server_table.sql"
    ),
    sha256: "ab8e6e84af7b3080d559c5e2023752b7f1bd69c94111284fe1d878bcdf4458ea",
};

const PINNED_TOKEN_EXCHANGE: MigrationFile = MigrationFile {
    directory: "20260630120000_add_token_exchange_to_mcp_servers",
    bytes: include_bytes!("fixtures/20260630120000_add_token_exchange_to_mcp_servers.sql"),
    sha256: "ddc9a89115f25ec043b6dfd46eff43e37289db55ab3e82a74290660d308a0399",
};

const PINNED_BUDGET_FALLBACKS: MigrationFile = MigrationFile {
    directory: "20260630190000_add_budget_fallbacks_to_litellm_verification_token",
    bytes: include_bytes!(
        "fixtures/20260630190000_add_budget_fallbacks_to_litellm_verification_token.sql"
    ),
    sha256: "e505c9f94735b209b790293b0a02e14a9b7fd306d144c0d6138190874105e02e",
};

const PINNED_EXCHANGE_PROFILE: MigrationFile = MigrationFile {
    directory: "20260703120000_add_token_exchange_profile_to_mcp_servers",
    bytes: include_bytes!("fixtures/20260703120000_add_token_exchange_profile_to_mcp_servers.sql"),
    sha256: "c17998f21e0122401c7f3e4d11f02b7c3924da6ae9a0f5a17bb4920d7b5b3ccc",
};

const PINNED_MCP_DCR_BRIDGE: MigrationFile = MigrationFile {
    directory: "20260710000000_add_dcr_bridge_to_mcp_server_table",
    bytes: include_bytes!("fixtures/20260710000000_add_dcr_bridge_to_mcp_server_table.sql"),
    sha256: "8071e33b4af00586d53adfecfac43a6ce3ece46aadabc306e43083c8c429faa4",
};

const PINNED_DAILY_TEAM_PTU: MigrationFile = MigrationFile {
    directory: "20260713010000_add_ptu_columns_to_daily_team_spend",
    bytes: include_bytes!("fixtures/20260713010000_add_ptu_columns_to_daily_team_spend.sql"),
    sha256: "b8b667c881330111a4f034e20d489b962b2c7599ca8ebbd54cadfcb9e7446076",
};

const PINNED_TOKEN_KEY_TYPE: MigrationFile = MigrationFile {
    directory: "20260713230852_add_key_type_to_litellm_verification_token",
    bytes: include_bytes!("fixtures/20260713230852_add_key_type_to_litellm_verification_token.sql"),
    sha256: "e8cf45e1f2aab2ddaad1d53cd67f518e08f803b409ccac210a9c3a3e8c6a03f5",
};

const PINNED_MCP_ISSUER: MigrationFile = MigrationFile {
    directory: "20260715000000_add_issuer_to_mcp_server_table",
    bytes: include_bytes!("fixtures/20260715000000_add_issuer_to_mcp_server_table.sql"),
    sha256: "bb5a30704d0ff8a32635b757a34027d84170ec0ad114bcaf1ca71e771f57b19a",
};

const PINNED_COMPRESSION: MigrationFile = MigrationFile {
    directory: "20260717000000_add_compression_saved_tokens",
    bytes: include_bytes!("fixtures/20260717000000_add_compression_saved_tokens.sql"),
    sha256: "3bd9818383657f7399aa7e5b5acb8743f3d684063cad4e0f962927f8574eefe0",
};

const PINNED_MCP_OAUTH_CLIENT: MigrationFile = MigrationFile {
    directory: "20260717000000_add_mcp_server_oauth_client_table",
    bytes: include_bytes!("fixtures/20260717000000_add_mcp_server_oauth_client_table.sql"),
    sha256: "9d9b8d465b5c7974d73798f07b74496a5ff5a615e9fc646c8eac3651f50eb94b",
};

const PINNED_JWT_KEY_CASCADE: MigrationFile = MigrationFile {
    directory: "20260717000000_cascade_delete_jwt_key_mapping_on_token_delete",
    bytes: include_bytes!(
        "fixtures/20260717000000_cascade_delete_jwt_key_mapping_on_token_delete.sql"
    ),
    sha256: "a9d57b2cfb9e268734ed4d43786f3423f333c51d8f6f698ec4a3b1ffb77c446a",
};

const PINNED_ADD_SAVINGS_SPEND: MigrationFile = MigrationFile {
    directory: "20260718000000_add_savings_spend",
    bytes: include_bytes!("fixtures/20260718000000_add_savings_spend.sql"),
    sha256: "09d40b5beacaf2c2bf1c73da2fe0e67e2812e3fd6ee1977c47ae9417eda0667d",
};

const PINNED_SSO_ASSERTION: MigrationFile = MigrationFile {
    directory: "20260721000000_add_sso_identity_assertion",
    bytes: include_bytes!("fixtures/20260721000000_add_sso_identity_assertion.sql"),
    sha256: "61df604554a40f55fd44a1f94d20b154eea3d10bc83a399d64ed42a784a37e71",
};

const PINNED_SPEND_LOG_INDEX: MigrationFile = MigrationFile {
    directory: "20260724000000_add_spend_log_tool_index_start_time_idx",
    bytes: include_bytes!("fixtures/20260724000000_add_spend_log_tool_index_start_time_idx.sql"),
    sha256: "3a73fe2955610df1fd635e2ab4ffb33004db5d0c03221eda799ac45455d7af09",
};

const PINNED_ADD_DAILY_TOOL_SPEND: MigrationFile = MigrationFile {
    directory: "20260725000000_add_daily_tool_spend",
    bytes: include_bytes!("fixtures/20260725000000_add_daily_tool_spend.sql"),
    sha256: "626702edb86035095d6ae9cd21ee218bc7ed98267c2472a9918b1f7057957ab2",
};

const PINNED_RELOAD_TRACKING: MigrationFile = MigrationFile {
    directory: "20260729000000_add_reload_tracking_to_litellm_config",
    bytes: include_bytes!("fixtures/20260729000000_add_reload_tracking_to_litellm_config.sql"),
    sha256: "c2b92f39fc51fc9a23ebc90eebc648e5cd1eaa70fcc92db7bb43cf1b6d9a2e7e",
};

const PINNED_OBJECT_TAGS: MigrationFile = MigrationFile {
    directory: "20260730000000_add_api_key_and_request_tags_to_managed_object_table",
    bytes: include_bytes!(
        "fixtures/20260730000000_add_api_key_and_request_tags_to_managed_object_table.sql"
    ),
    sha256: "dbfd2ddf670c8ee0d6016c38db9858334d2593f739e0778ccf60691e645819e1",
};

const PINNED_AUTOROUTER_SPEND: MigrationFile = MigrationFile {
    directory: "20260731000000_add_autorouter_savings_spend",
    bytes: include_bytes!("fixtures/20260731000000_add_autorouter_savings_spend.sql"),
    sha256: "7bb2048d3377d9a237a5650f757992aada6bc9d88a003b326d283397541f330e",
};

const PINNED_GATEWAY_REQUESTS: MigrationFile = MigrationFile {
    directory: "20260803000000_add_daily_gateway_requests",
    bytes: include_bytes!("fixtures/20260803000000_add_daily_gateway_requests.sql"),
    sha256: "d052bf6f7fead1abbe9dce9b7adda7a6497acd685321f91b819ecd1ac34eb551",
};

const PINNED_BUDGET_WINDOW: MigrationFile = MigrationFile {
    directory: "20260804162853_add_budget_window_spend_table",
    bytes: include_bytes!("fixtures/20260804162853_add_budget_window_spend_table.sql"),
    sha256: "ea19ecfef5e181f8b4ac20d9e5baeb241c2d10a793de3c50cfc24b69ca600774",
};

const PINNED_SESSION_ROLLUP: MigrationFile = MigrationFile {
    directory: "20260805000000_add_autorouter_session_rollup",
    bytes: include_bytes!("fixtures/20260805000000_add_autorouter_session_rollup.sql"),
    sha256: "25607951a7a70e13670a7209c1b711cf22e758b441028b4ce6d9e76c662d184f",
};

const PINNED_AUTOROUTER_TURNS: MigrationFile = MigrationFile {
    directory: "20260807000000_add_autorouter_session_tier_turns",
    bytes: include_bytes!("fixtures/20260807000000_add_autorouter_session_tier_turns.sql"),
    sha256: "22e5b257dcd2d8695f9b76eddc5f6543a245fdcea14bf5ba6f0643eebb988a54",
};

const PINNED_TOKEN_SETTINGS: MigrationFile = MigrationFile {
    directory: "20260810000000_add_verificationtoken_settings_updated_at",
    bytes: include_bytes!("fixtures/20260810000000_add_verificationtoken_settings_updated_at.sql"),
    sha256: "420c8634a5febc76b118fcb9295b0dfd73579d58396ac85746f4098c46d34739",
};

pub(super) const PINNED_MIGRATION_CATALOG: MigrationCatalog = MigrationCatalog {
    files: &[
        PINNED_ADAPTIVE_ROUTER,
        PINNED_ADD_MEMORY_TABLE,
        PINNED_TEAM_TOTAL_SPEND,
        PINNED_TOOLS_OBJ_PERM,
        PINNED_WORKFLOW_RUNS_TABLES,
        PINNED_RESOURCE_OWNER,
        PINNED_DELEGATE_AUTH,
        PINNED_PROXY_BLOCKED,
        PINNED_ADD_MCP_ENV_VARS,
        PINNED_OAUTH_PASSTHRU,
        PINNED_OAUTH2_FLOW,
        PINNED_MCP_TIMEOUT,
        PINNED_MCP_TOOL_SEARCH,
        PINNED_MCP_CONCURRENCY,
        PINNED_TOKEN_EXCHANGE,
        PINNED_BUDGET_FALLBACKS,
        PINNED_EXCHANGE_PROFILE,
        PINNED_MCP_DCR_BRIDGE,
        PINNED_DAILY_TEAM_PTU,
        PINNED_TOKEN_KEY_TYPE,
        PINNED_MCP_ISSUER,
        PINNED_COMPRESSION,
        PINNED_MCP_OAUTH_CLIENT,
        PINNED_JWT_KEY_CASCADE,
        PINNED_ADD_SAVINGS_SPEND,
        PINNED_SSO_ASSERTION,
        PINNED_SPEND_LOG_INDEX,
        PINNED_ADD_DAILY_TOOL_SPEND,
        PINNED_RELOAD_TRACKING,
        PINNED_OBJECT_TAGS,
        PINNED_AUTOROUTER_SPEND,
        PINNED_GATEWAY_REQUESTS,
        PINNED_BUDGET_WINDOW,
        PINNED_SESSION_ROLLUP,
        PINNED_AUTOROUTER_TURNS,
        PINNED_TOKEN_SETTINGS,
    ],
};
