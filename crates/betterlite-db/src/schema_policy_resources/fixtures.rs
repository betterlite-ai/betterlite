#![cfg(test)]

use crate::schema_baseline::{MigrationCatalog, MigrationFile};

const PINNED_BASELINE_DIFF: MigrationFile = MigrationFile {
    directory: "20260219181415_baseline_diff",
    bytes: include_bytes!("fixtures/20260219181415_baseline_diff.sql"),
    sha256: "ecbd073ba23472f98ede77177d4318abb9ae0958bae691ed3939dead91bd299e",
};

const PINNED_SPEC_PATH_ADDED: MigrationFile = MigrationFile {
    directory: "20260220124742_add_spec_path_to_mcp_servers",
    bytes: include_bytes!("fixtures/20260220124742_add_spec_path_to_mcp_servers.sql"),
    sha256: "e49454e7f5be2ba0e46da7e5be61900942398f971dcfb0ab70ff4fcef9cfd50e",
};

const PINNED_INDEX_AGGREGATES: MigrationFile = MigrationFile {
    directory: "20260220153844_add_composite_index_aggregate_tables",
    bytes: include_bytes!("fixtures/20260220153844_add_composite_index_aggregate_tables.sql"),
    sha256: "3f509af1ecb34b350a56f8e35ef47944541f487ddf5033e70f05c2f0ebcc0615",
};

const PINNED_PROJECT_ID_VERIFY: MigrationFile = MigrationFile {
    directory: "20260221000000_ensure_project_id_verification_token",
    bytes: include_bytes!("fixtures/20260221000000_ensure_project_id_verification_token.sql"),
    sha256: "c6e0c6f91022ecb66af31eb3778913a2313b1b8270dd7f241681abf354ce306d",
};

const PINNED_ADD_POLICY_VERSIONING: MigrationFile = MigrationFile {
    directory: "20260221183800_add_policy_versioning",
    bytes: include_bytes!("fixtures/20260221183800_add_policy_versioning.sql"),
    sha256: "7f3f29b50badc248a70f5a4aaee3ed06d933e1ad8ebaef543338a6cb816faf4b",
};

const PINNED_BATCH_PROCESSED: MigrationFile = MigrationFile {
    directory: "20260222000000_add_batch_processed_to_managed_object_table",
    bytes: include_bytes!(
        "fixtures/20260222000000_add_batch_processed_to_managed_object_table.sql"
    ),
    sha256: "70826378aec43d18575c23e83417bc11455a267104eb0909918a96a24118ba55",
};

const PINNED_SPEND_LOG_DURATION: MigrationFile = MigrationFile {
    directory: "20260224201417_spend_logs_request_duration",
    bytes: include_bytes!("fixtures/20260224201417_spend_logs_request_duration.sql"),
    sha256: "bc0cb6ca3c8f432c0b32ad128fa3667ed991245b672e62e9fd18ca34552482ee",
};

const PINNED_AGENT_OBJ_PERMISSIONS: MigrationFile = MigrationFile {
    directory: "20260224203854_add_agent_object_permissions_table",
    bytes: include_bytes!("fixtures/20260224203854_add_agent_object_permissions_table.sql"),
    sha256: "c7d02b13901092b4f9bcc4eac1102f23ba8d639ed3fa9745277647df904afb92",
};

const PINNED_BLOCKED_TOOLS_PERM: MigrationFile = MigrationFile {
    directory: "20260226000000_add_blocked_tools_to_object_permission",
    bytes: include_bytes!("fixtures/20260226000000_add_blocked_tools_to_object_permission.sql"),
    sha256: "855545571013964f192ec7d066c001392d2778c47ad1306c974322cc4ad7482f",
};

const PINNED_SPEND_TOOL_INDEX: MigrationFile = MigrationFile {
    directory: "20260226120000_add_spend_log_tool_index",
    bytes: include_bytes!("fixtures/20260226120000_add_spend_log_tool_index.sql"),
    sha256: "019ae3e4e71f35763d6d515e1d463d97e75b8c50e7637c842e4e16ce59009a4b",
};

const PINNED_AGENT_ID_DELETE_KEYS: MigrationFile = MigrationFile {
    directory: "20260226202727_add_agent_id_to_delete_keys",
    bytes: include_bytes!("fixtures/20260226202727_add_agent_id_to_delete_keys.sql"),
    sha256: "67af990c525e2160cbda0c759d6992ccbfc4b228313c2439b3b47f93c5c4c13c",
};

const PINNED_CLAUDE_PLUGIN: MigrationFile = MigrationFile {
    directory: "20260228000000_add_claude_code_plugin_table",
    bytes: include_bytes!("fixtures/20260228000000_add_claude_code_plugin_table.sql"),
    sha256: "97f41d1817251781594a1dbf5d812f1a860ff21da79d11e914d5151179481fd4",
};

const PINNED_SPEND_LOGS_INDEX: MigrationFile = MigrationFile {
    directory: "20260228100000_add_spend_logs_composite_index",
    bytes: include_bytes!("fixtures/20260228100000_add_spend_logs_composite_index.sql"),
    sha256: "7ece8b0cdb67336e9914f87882bcc804ada60de54a715f10175a9450748d6cfa",
};

const PINNED_MCP_PUBLIC_INTERNET: MigrationFile = MigrationFile {
    directory: "20260228110000_mcp_default_public_internet_true",
    bytes: include_bytes!("fixtures/20260228110000_mcp_default_public_internet_true.sql"),
    sha256: "e007fdb13fe6128d5ac59ae7e7c4b92b08a147a38a44ddcf518dbe94d3904726",
};

const PINNED_TEAM_GUARDRAILS: MigrationFile = MigrationFile {
    directory: "20260228170127_support_team_based_guardrails",
    bytes: include_bytes!("fixtures/20260228170127_support_team_based_guardrails.sql"),
    sha256: "3610ed4728cd666030881bc61380441901e65a0705296ed255a1bc5001bf6457",
};

const PINNED_TOOL_TABLE_POLICIES: MigrationFile = MigrationFile {
    directory: "20260303000000_update_tool_table_policies",
    bytes: include_bytes!("fixtures/20260303000000_update_tool_table_policies.sql"),
    sha256: "e85f13e8222e6aa682cf1c8e8af45823bf480d85446893c71d77fbea18adcd61",
};

const PINNED_AGENT_SPEND: MigrationFile = MigrationFile {
    directory: "20260304175016_add_spend_to_agent_table",
    bytes: include_bytes!("fixtures/20260304175016_add_spend_to_agent_table.sql"),
    sha256: "cbdcbf988896f0ba08bc774f345f550ea3c784c4bc5eda872af093be346cc84d",
};

const PINNED_ADD_AGENT_HEADERS: MigrationFile = MigrationFile {
    directory: "20260305000000_add_agent_headers",
    bytes: include_bytes!("fixtures/20260305000000_add_agent_headers.sql"),
    sha256: "225bc1c1da92e33d0088e7703fe786430da76cc631317eaef1d4aecc520f8262",
};

const PINNED_AGENT_RATE_LIMITS: MigrationFile = MigrationFile {
    directory: "20260305000000_add_rate_limits_to_agents",
    bytes: include_bytes!("fixtures/20260305000000_add_rate_limits_to_agents.sql"),
    sha256: "ebe9b00cdbe4225e49f9bba8d0dcf1e22dc7b43f63bb5ccebc8fdde76f227f14",
};

const PINNED_CONFIG_OVERRIDE: MigrationFile = MigrationFile {
    directory: "20260306175056_add_configs_override_table",
    bytes: include_bytes!("fixtures/20260306175056_add_configs_override_table.sql"),
    sha256: "f4b1682d40d1999b98215a0677481980372e4cb5659d071e878a343a7a65944b",
};

const PINNED_SCHEMA_SYNC_20260306: MigrationFile = MigrationFile {
    directory: "20260306233848_schema_sync",
    bytes: include_bytes!("fixtures/20260306233848_schema_sync.sql"),
    sha256: "1fac2f1e9d937b170c06b7828f3de1290d056287d81901b03c174d75bf1e0604",
};

const PINNED_ADD_MCP_APPROVAL_STATUS: MigrationFile = MigrationFile {
    directory: "20260309000000_add_mcp_approval_status",
    bytes: include_bytes!("fixtures/20260309000000_add_mcp_approval_status.sql"),
    sha256: "69c77e41e45c3f24f50bfccc2801795aa9ecbdada2cc35263f1977a81457d55a",
};

const PINNED_ADD_MCP_SOURCE_URL: MigrationFile = MigrationFile {
    directory: "20260309000001_add_mcp_source_url",
    bytes: include_bytes!("fixtures/20260309000001_add_mcp_source_url.sql"),
    sha256: "cbf048fe8db758b349f53a1d4e25107d354a41c3aa9080bb1d5a091343e125b0",
};

const PINNED_SCHEMA_SYNC_20260312: MigrationFile = MigrationFile {
    directory: "20260312124619_schema_sync",
    bytes: include_bytes!("fixtures/20260312124619_schema_sync.sql"),
    sha256: "62bc045f03c1393c83743262d885693297f891b879bcbf11c749c588e005525b",
};

const PINNED_ADD_INDEX_TO_TEAM_TABLE: MigrationFile = MigrationFile {
    directory: "20260318140652_add_index_to_team_table",
    bytes: include_bytes!("fixtures/20260318140652_add_index_to_team_table.sql"),
    sha256: "6d2aa9c28c9c3de83bd5f6cee2bdc6a2b671f348ca8dee58dfc7ffd2abf831cb",
};

const PINNED_MCP_APPROVAL_RESTORE: MigrationFile = MigrationFile {
    directory: "20260319000000_restore_mcp_approval_fields",
    bytes: include_bytes!("fixtures/20260319000000_restore_mcp_approval_fields.sql"),
    sha256: "552e04c0e6590ef1b2e9d0e1a181e3c98d83581ae57ab4e292aafa76a54af1fb",
};

const PINNED_ADD_MCP_TOOLSETS: MigrationFile = MigrationFile {
    directory: "20260321000000_add_mcp_toolsets",
    bytes: include_bytes!("fixtures/20260321000000_add_mcp_toolsets.sql"),
    sha256: "4b151c1a923781877f3666157d5463a1e0c565dad6c20e73eb8840b76c5bf9a0",
};

const PINNED_PROMPT_ENV_CREATED_BY: MigrationFile = MigrationFile {
    directory: "20260331000000_add_prompt_environment_and_created_by",
    bytes: include_bytes!("fixtures/20260331000000_add_prompt_environment_and_created_by.sql"),
    sha256: "52db3f7f88c3073debc15024e2d3c448a294510531d5189a9fe7d6cbfc44756b",
};

const PINNED_ADD_BUDGET_LIMITS: MigrationFile = MigrationFile {
    directory: "20260401000000_add_budget_limits",
    bytes: include_bytes!("fixtures/20260401000000_add_budget_limits.sql"),
    sha256: "ef1161b00f477701b08b9868c8ca4b41266765a946c3adda8530d810db7bc7c5",
};

const PINNED_TEAM_MODEL_SCOPE: MigrationFile = MigrationFile {
    directory: "20260401000000_add_team_member_model_scope",
    bytes: include_bytes!("fixtures/20260401000000_add_team_member_model_scope.sql"),
    sha256: "4872cb4220d148cc96b4a17e8673566891ebdd804a576112db705147bc9c1a2f",
};

const PINNED_MCP_INSTRUCTIONS: MigrationFile = MigrationFile {
    directory: "20260414140000_add_mcp_server_instructions",
    bytes: include_bytes!("fixtures/20260414140000_add_mcp_server_instructions.sql"),
    sha256: "076b759528c1065b535472c6694bbdeb7d6b0b5446f3df26ea68aac3294c49b1",
};

const PINNED_HEALTH_CHECK_LATEST: MigrationFile = MigrationFile {
    directory: "20260415120000_health_check_latest_per_model_index",
    bytes: include_bytes!("fixtures/20260415120000_health_check_latest_per_model_index.sql"),
    sha256: "e5dce697bc2b61744f058f64892ebd7ef4112a46fc82ba42cd6104785e977ccb",
};

pub(super) const PINNED_MIGRATION_CATALOG: MigrationCatalog = MigrationCatalog {
    files: &[
        PINNED_BASELINE_DIFF,
        PINNED_SPEC_PATH_ADDED,
        PINNED_INDEX_AGGREGATES,
        PINNED_PROJECT_ID_VERIFY,
        PINNED_ADD_POLICY_VERSIONING,
        PINNED_BATCH_PROCESSED,
        PINNED_SPEND_LOG_DURATION,
        PINNED_AGENT_OBJ_PERMISSIONS,
        PINNED_BLOCKED_TOOLS_PERM,
        PINNED_SPEND_TOOL_INDEX,
        PINNED_AGENT_ID_DELETE_KEYS,
        PINNED_CLAUDE_PLUGIN,
        PINNED_SPEND_LOGS_INDEX,
        PINNED_MCP_PUBLIC_INTERNET,
        PINNED_TEAM_GUARDRAILS,
        PINNED_TOOL_TABLE_POLICIES,
        PINNED_AGENT_SPEND,
        PINNED_ADD_AGENT_HEADERS,
        PINNED_AGENT_RATE_LIMITS,
        PINNED_CONFIG_OVERRIDE,
        PINNED_SCHEMA_SYNC_20260306,
        PINNED_ADD_MCP_APPROVAL_STATUS,
        PINNED_ADD_MCP_SOURCE_URL,
        PINNED_SCHEMA_SYNC_20260312,
        PINNED_ADD_INDEX_TO_TEAM_TABLE,
        PINNED_MCP_APPROVAL_RESTORE,
        PINNED_ADD_MCP_TOOLSETS,
        PINNED_PROMPT_ENV_CREATED_BY,
        PINNED_ADD_BUDGET_LIMITS,
        PINNED_TEAM_MODEL_SCOPE,
        PINNED_MCP_INSTRUCTIONS,
        PINNED_HEALTH_CHECK_LATEST,
    ],
};
