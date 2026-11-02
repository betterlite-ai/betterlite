use crate::schema_baseline::{MigrationCatalog, MigrationFile};

pub mod fixtures;

pub const BASELINE_DIFF: MigrationFile = MigrationFile {
    directory: "20260219181415_baseline_diff",
    bytes: include_bytes!("../migrations/20260219181415_baseline_diff/migration.sql"),
    sha256: "d2baed7a684683cf7163aa52aafb8c3fa6d2d8b9ae4641c8408a4afea3ee7015",
};

pub const SPEC_PATH_ADDED: MigrationFile = MigrationFile {
    directory: "20260220124742_add_spec_path_to_mcp_servers",
    bytes: include_bytes!(
        "../migrations/20260220124742_add_spec_path_to_mcp_servers/migration.sql"
    ),
    sha256: "e49454e7f5be2ba0e46da7e5be61900942398f971dcfb0ab70ff4fcef9cfd50e",
};

pub const COMPOSITE_INDEX_AGGREGATES: MigrationFile = MigrationFile {
    directory: "20260220153844_add_composite_index_aggregate_tables",
    bytes: include_bytes!(
        "../migrations/20260220153844_add_composite_index_aggregate_tables/migration.sql"
    ),
    sha256: "bfa64602f3049c668138388e0dede890cf9bad41c0ced302ed69ebceef0ca8ce",
};

pub const ENSURE_PROJECT_ID_VERIFY: MigrationFile = MigrationFile {
    directory: "20260221000000_ensure_project_id_verification_token",
    bytes: include_bytes!(
        "../migrations/20260221000000_ensure_project_id_verification_token/migration.sql"
    ),
    sha256: "c6e0c6f91022ecb66af31eb3778913a2313b1b8270dd7f241681abf354ce306d",
};

pub const ADD_POLICY_VERSIONING: MigrationFile = MigrationFile {
    directory: "20260221183800_add_policy_versioning",
    bytes: include_bytes!("../migrations/20260221183800_add_policy_versioning/migration.sql"),
    sha256: "e83e6e5c2e2f8f2f84e3f1b3e13798836380fcee87d02a06d1c839b3319a602e",
};

pub const ADD_BATCH_PROCESSED_MANAGED: MigrationFile = MigrationFile {
    directory: "20260222000000_add_batch_processed_to_managed_object_table",
    bytes: include_bytes!(
        "../migrations/20260222000000_add_batch_processed_to_managed_object_table/migration.sql"
    ),
    sha256: "70826378aec43d18575c23e83417bc11455a267104eb0909918a96a24118ba55",
};

pub const SPEND_LOGS_REQUEST_DURATION: MigrationFile = MigrationFile {
    directory: "20260224201417_spend_logs_request_duration",
    bytes: include_bytes!("../migrations/20260224201417_spend_logs_request_duration/migration.sql"),
    sha256: "937da574e62e90a8bc308200a97526759b8c952bf1f8bc22e7185ee945f8226f",
};

pub const ADD_AGENT_OBJECT_PERMISSIONS: MigrationFile = MigrationFile {
    directory: "20260224203854_add_agent_object_permissions_table",
    bytes: include_bytes!(
        "../migrations/20260224203854_add_agent_object_permissions_table/migration.sql"
    ),
    sha256: "ba3852c3fed43e7e2553073e1b8452d93db8784ef0bb733bc6cc821f0f650adc",
};

pub const ADD_BLOCKED_TOOLS_PERMISSION: MigrationFile = MigrationFile {
    directory: "20260226000000_add_blocked_tools_to_object_permission",
    bytes: include_bytes!(
        "../migrations/20260226000000_add_blocked_tools_to_object_permission/migration.sql"
    ),
    sha256: "855545571013964f192ec7d066c001392d2778c47ad1306c974322cc4ad7482f",
};

pub const ADD_SPEND_LOG_TOOL_INDEX: MigrationFile = MigrationFile {
    directory: "20260226120000_add_spend_log_tool_index",
    bytes: include_bytes!("../migrations/20260226120000_add_spend_log_tool_index/migration.sql"),
    sha256: "019ae3e4e71f35763d6d515e1d463d97e75b8c50e7637c842e4e16ce59009a4b",
};

pub const ADD_AGENT_ID_TO_DELETE_KEYS: MigrationFile = MigrationFile {
    directory: "20260226202727_add_agent_id_to_delete_keys",
    bytes: include_bytes!("../migrations/20260226202727_add_agent_id_to_delete_keys/migration.sql"),
    sha256: "745a05b983d4123a9b7fdbc90ae73333c76aef3fe67b2e47cee979cf418a771b",
};

pub const ADD_CLAUDE_CODE_PLUGIN_TABLE: MigrationFile = MigrationFile {
    directory: "20260228000000_add_claude_code_plugin_table",
    bytes: include_bytes!(
        "../migrations/20260228000000_add_claude_code_plugin_table/migration.sql"
    ),
    sha256: "97f41d1817251781594a1dbf5d812f1a860ff21da79d11e914d5151179481fd4",
};

pub const ADD_SPEND_LOGS_COMPOSITE_INDEX: MigrationFile = MigrationFile {
    directory: "20260228100000_add_spend_logs_composite_index",
    bytes: include_bytes!(
        "../migrations/20260228100000_add_spend_logs_composite_index/migration.sql"
    ),
    sha256: "7ece8b0cdb67336e9914f87882bcc804ada60de54a715f10175a9450748d6cfa",
};

pub const MCP_DEFAULT_PUBLIC_INTERNET: MigrationFile = MigrationFile {
    directory: "20260228110000_mcp_default_public_internet_true",
    bytes: include_bytes!(
        "../migrations/20260228110000_mcp_default_public_internet_true/migration.sql"
    ),
    sha256: "e007fdb13fe6128d5ac59ae7e7c4b92b08a147a38a44ddcf518dbe94d3904726",
};

pub const SUPPORT_TEAM_BASED_GUARDRAILS: MigrationFile = MigrationFile {
    directory: "20260228170127_support_team_based_guardrails",
    bytes: include_bytes!(
        "../migrations/20260228170127_support_team_based_guardrails/migration.sql"
    ),
    sha256: "a3406ec586de8f3dfca15098f0006439d63002e9fa80a1f915d4ff3392502d1e",
};

pub const UPDATE_TOOL_TABLE_POLICIES: MigrationFile = MigrationFile {
    directory: "20260303000000_update_tool_table_policies",
    bytes: include_bytes!("../migrations/20260303000000_update_tool_table_policies/migration.sql"),
    sha256: "e85f13e8222e6aa682cf1c8e8af45823bf480d85446893c71d77fbea18adcd61",
};

pub const ADD_SPEND_TO_AGENT_TABLE: MigrationFile = MigrationFile {
    directory: "20260304175016_add_spend_to_agent_table",
    bytes: include_bytes!("../migrations/20260304175016_add_spend_to_agent_table/migration.sql"),
    sha256: "9f9c059ded60570b218dda9b928639f0e9f9b6e8467e24a8a344f1a0d2067a61",
};

pub const ADD_AGENT_HEADERS: MigrationFile = MigrationFile {
    directory: "20260305000000_add_agent_headers",
    bytes: include_bytes!("../migrations/20260305000000_add_agent_headers/migration.sql"),
    sha256: "225bc1c1da92e33d0088e7703fe786430da76cc631317eaef1d4aecc520f8262",
};

pub const ADD_RATE_LIMITS_TO_AGENTS: MigrationFile = MigrationFile {
    directory: "20260305000000_add_rate_limits_to_agents",
    bytes: include_bytes!("../migrations/20260305000000_add_rate_limits_to_agents/migration.sql"),
    sha256: "ebe9b00cdbe4225e49f9bba8d0dcf1e22dc7b43f63bb5ccebc8fdde76f227f14",
};

pub const ADD_CONFIGS_OVERRIDE_TABLE: MigrationFile = MigrationFile {
    directory: "20260306175056_add_configs_override_table",
    bytes: include_bytes!("../migrations/20260306175056_add_configs_override_table/migration.sql"),
    sha256: "e49454e7f5be2ba0e46da7e5be61900942398f971dcfb0ab70ff4fcef9cfd50e",
};

pub const SCHEMA_SYNC_20260306: MigrationFile = MigrationFile {
    directory: "20260306233848_schema_sync",
    bytes: include_bytes!("../migrations/20260306233848_schema_sync/migration.sql"),
    sha256: "1c8a8902aaad9787995e564e895577e22818ed794a37d2a9e14e68ce74c7fd24",
};

pub const ADD_MCP_APPROVAL_STATUS: MigrationFile = MigrationFile {
    directory: "20260309000000_add_mcp_approval_status",
    bytes: include_bytes!("../migrations/20260309000000_add_mcp_approval_status/migration.sql"),
    sha256: "69c77e41e45c3f24f50bfccc2801795aa9ecbdada2cc35263f1977a81457d55a",
};

pub const ADD_MCP_SOURCE_URL: MigrationFile = MigrationFile {
    directory: "20260309000001_add_mcp_source_url",
    bytes: include_bytes!("../migrations/20260309000001_add_mcp_source_url/migration.sql"),
    sha256: "cbf048fe8db758b349f53a1d4e25107d354a41c3aa9080bb1d5a091343e125b0",
};

pub const SCHEMA_SYNC_20260312: MigrationFile = MigrationFile {
    directory: "20260312124619_schema_sync",
    bytes: include_bytes!("../migrations/20260312124619_schema_sync/migration.sql"),
    sha256: "094fbe1bf41349e54468d0ad7f94570d0a63dee116fa804405e5ecb49fd4b197",
};

pub const ADD_INDEX_TO_TEAM_TABLE: MigrationFile = MigrationFile {
    directory: "20260318140652_add_index_to_team_table",
    bytes: include_bytes!("../migrations/20260318140652_add_index_to_team_table/migration.sql"),
    sha256: "1a639b962624ca135c2e89457fd8868f384c96b96e0bbc6057522bd04c1d9b3f",
};

pub const RESTORE_MCP_APPROVAL_FIELDS: MigrationFile = MigrationFile {
    directory: "20260319000000_restore_mcp_approval_fields",
    bytes: include_bytes!("../migrations/20260319000000_restore_mcp_approval_fields/migration.sql"),
    sha256: "552e04c0e6590ef1b2e9d0e1a181e3c98d83581ae57ab4e292aafa76a54af1fb",
};

pub const ADD_MCP_TOOLSETS: MigrationFile = MigrationFile {
    directory: "20260321000000_add_mcp_toolsets",
    bytes: include_bytes!("../migrations/20260321000000_add_mcp_toolsets/migration.sql"),
    sha256: "4b151c1a923781877f3666157d5463a1e0c565dad6c20e73eb8840b76c5bf9a0",
};

pub const ADD_PROMPT_ENV_CREATED_BY: MigrationFile = MigrationFile {
    directory: "20260331000000_add_prompt_environment_and_created_by",
    bytes: include_bytes!(
        "../migrations/20260331000000_add_prompt_environment_and_created_by/migration.sql"
    ),
    sha256: "52db3f7f88c3073debc15024e2d3c448a294510531d5189a9fe7d6cbfc44756b",
};

pub const ADD_BUDGET_LIMITS: MigrationFile = MigrationFile {
    directory: "20260401000000_add_budget_limits",
    bytes: include_bytes!("../migrations/20260401000000_add_budget_limits/migration.sql"),
    sha256: "ef1161b00f477701b08b9868c8ca4b41266765a946c3adda8530d810db7bc7c5",
};

pub const ADD_TEAM_MEMBER_MODEL_SCOPE: MigrationFile = MigrationFile {
    directory: "20260401000000_add_team_member_model_scope",
    bytes: include_bytes!("../migrations/20260401000000_add_team_member_model_scope/migration.sql"),
    sha256: "4872cb4220d148cc96b4a17e8673566891ebdd804a576112db705147bc9c1a2f",
};

pub const ADD_MCP_SERVER_INSTRUCTIONS: MigrationFile = MigrationFile {
    directory: "20260414140000_add_mcp_server_instructions",
    bytes: include_bytes!("../migrations/20260414140000_add_mcp_server_instructions/migration.sql"),
    sha256: "076b759528c1065b535472c6694bbdeb7d6b0b5446f3df26ea68aac3294c49b1",
};

pub const HEALTH_CHECK_LATEST_PER_MODEL: MigrationFile = MigrationFile {
    directory: "20260415120000_health_check_latest_per_model_index",
    bytes: include_bytes!(
        "../migrations/20260415120000_health_check_latest_per_model_index/migration.sql"
    ),
    sha256: "e5dce697bc2b61744f058f64892ebd7ef4112a46fc82ba42cd6104785e977ccb",
};

pub const MIGRATION_CATALOG: MigrationCatalog = MigrationCatalog {
    files: &[
        BASELINE_DIFF,
        SPEC_PATH_ADDED,
        COMPOSITE_INDEX_AGGREGATES,
        ENSURE_PROJECT_ID_VERIFY,
        ADD_POLICY_VERSIONING,
        ADD_BATCH_PROCESSED_MANAGED,
        SPEND_LOGS_REQUEST_DURATION,
        ADD_AGENT_OBJECT_PERMISSIONS,
        ADD_BLOCKED_TOOLS_PERMISSION,
        ADD_SPEND_LOG_TOOL_INDEX,
        ADD_AGENT_ID_TO_DELETE_KEYS,
        ADD_CLAUDE_CODE_PLUGIN_TABLE,
        ADD_SPEND_LOGS_COMPOSITE_INDEX,
        MCP_DEFAULT_PUBLIC_INTERNET,
        SUPPORT_TEAM_BASED_GUARDRAILS,
        UPDATE_TOOL_TABLE_POLICIES,
        ADD_SPEND_TO_AGENT_TABLE,
        ADD_AGENT_HEADERS,
        ADD_RATE_LIMITS_TO_AGENTS,
        ADD_CONFIGS_OVERRIDE_TABLE,
        SCHEMA_SYNC_20260306,
        ADD_MCP_APPROVAL_STATUS,
        ADD_MCP_SOURCE_URL,
        SCHEMA_SYNC_20260312,
        ADD_INDEX_TO_TEAM_TABLE,
        RESTORE_MCP_APPROVAL_FIELDS,
        ADD_MCP_TOOLSETS,
        ADD_PROMPT_ENV_CREATED_BY,
        ADD_BUDGET_LIMITS,
        ADD_TEAM_MEMBER_MODEL_SCOPE,
        ADD_MCP_SERVER_INSTRUCTIONS,
        HEALTH_CHECK_LATEST_PER_MODEL,
    ],
};

mod tests;
