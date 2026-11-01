use crate::schema_baseline::{MigrationCatalog, MigrationFile};

pub mod fixtures;

pub const ADD_MCP_NAMESPACED_TOOL_NAME: MigrationFile = MigrationFile {
    directory: "20250707230009_add_mcp_namespaced_tool_name",
    bytes: include_bytes!(
        "../migrations/20250707230009_add_mcp_namespaced_tool_name/migration.sql"
    ),
    sha256: "450f3dec527556576cd01903135789c5b8fbcc30d53dc1a6da2a55c832fc9b71",
};

pub const ADD_STDIO_MCP: MigrationFile = MigrationFile {
    directory: "20250711220620_add_stdio_mcp",
    bytes: include_bytes!("../migrations/20250711220620_add_stdio_mcp/migration.sql"),
    sha256: "b8b47e6aefd2932a04870838e452ef8a238ce95e3fab42d503d23db1fe156c07",
};

pub const LITELLM_PARAMS_VECTOR_STORES: MigrationFile = MigrationFile {
    directory: "20250718125714_add_litellm_params_to_vector_stores",
    bytes: include_bytes!(
        "../migrations/20250718125714_add_litellm_params_to_vector_stores/migration.sql"
    ),
    sha256: "8355d5360db519efb75a58bbbf87ca140e9e4ef07c281f1427109760ef9e103a",
};

pub const PROMPT_TABLE: MigrationFile = MigrationFile {
    directory: "20250802162330_prompt_table",
    bytes: include_bytes!("../migrations/20250802162330_prompt_table/migration.sql"),
    sha256: "fc049cda2848f0a1d0bb2df084cd6f51a0b0800e3ed893b5d19ecb0b73d75385",
};

pub const RENAME_ALIAS_TO_SERVER_NAME: MigrationFile = MigrationFile {
    directory: "20250806095134_rename_alias_to_server_name_mcp_table",
    bytes: include_bytes!(
        "../migrations/20250806095134_rename_alias_to_server_name_mcp_table/migration.sql"
    ),
    sha256: "1d6c391820bd439a63e5e22232b8a799f684d00ffa47d5148e28c42b2fb4d220",
};

pub const DROP_SPEC_VERSION_COLUMN: MigrationFile = MigrationFile {
    directory: "20250918083359_drop_spec_version_column_from_mcp_table",
    bytes: include_bytes!(
        "../migrations/20250918083359_drop_spec_version_column_from_mcp_table/migration.sql"
    ),
    sha256: "167f13c98e16513fc132e7586cc361a2cbcec7f322f3c8393ffa25d68e842d3e",
};

pub const UNNAMED_MIGRATION: MigrationFile = MigrationFile {
    directory: "20250926194702_unnamed_migration",
    bytes: include_bytes!("../migrations/20250926194702_unnamed_migration/migration.sql"),
    sha256: "8448eaba99b22e979f94708358c733356c434f6a2693c450bc2a90ba5bf5f68e",
};

pub const ADD_ALLOWED_TOOLS_TO_MCP: MigrationFile = MigrationFile {
    directory: "20251003165142_add_allowed_tools_to_mcp",
    bytes: include_bytes!("../migrations/20251003165142_add_allowed_tools_to_mcp/migration.sql"),
    sha256: "01648c55d6fb12c40ee480c0eee0dff74c3e822e5b0ce080370e66c4a6fe45f4",
};

pub const EXTRA_HEADERS_TO_MCP_TABLE: MigrationFile = MigrationFile {
    directory: "20251003190954_extra_headers_to_mcp_table",
    bytes: include_bytes!("../migrations/20251003190954_extra_headers_to_mcp_table/migration.sql"),
    sha256: "12823115d31ce16d301fe05fb6087a313d5bb7192c16862258dee9dd72df2827",
};

pub const ADD_MCP_TOOL_PERMISSIONS: MigrationFile = MigrationFile {
    directory: "20251006143948_add_mcp_tool_permissions",
    bytes: include_bytes!("../migrations/20251006143948_add_mcp_tool_permissions/migration.sql"),
    sha256: "49c4433b8d0f2e704e9deed9e8681fd5dd5266d201b2dfb9790e185ceec088f1",
};

pub const ADD_TAG_TABLE: MigrationFile = MigrationFile {
    directory: "20251011084309_add_tag_table",
    bytes: include_bytes!("../migrations/20251011084309_add_tag_table/migration.sql"),
    sha256: "67562e9108e7f68a79c460f933f9812cfa27964325442ed14c8a2ad02dfc5cf2",
};

pub const ADD_SEARCH_TOOL_TABLE: MigrationFile = MigrationFile {
    directory: "20251023141814_add_search_tool_table",
    bytes: include_bytes!("../migrations/20251023141814_add_search_tool_table/migration.sql"),
    sha256: "c9fd5a67fcb8225110f1dc667aae1236c5d7c569e902fbc2d9562cc5f60248bb",
};

pub const ADD_CACHE_CONFIG_TABLE: MigrationFile = MigrationFile {
    directory: "20251031181430_add_cache_config_table",
    bytes: include_bytes!("../migrations/20251031181430_add_cache_config_table/migration.sql"),
    sha256: "f371fc4d31f3b12ebe018f9a1c58f59f7f51d8dcb1ec3347f921a495958f9e45",
};

pub const MANAGED_VECTOR_STORE_INDEX: MigrationFile = MigrationFile {
    directory: "20251101131415_add_managed_vector_store_index_table",
    bytes: include_bytes!(
        "../migrations/20251101131415_add_managed_vector_store_index_table/migration.sql"
    ),
    sha256: "c55dda374f1b3e55e31ac27a627153dcc93d78d898b791aa2ba50f791e0be770",
};

pub const ADD_STATIC_HEADERS: MigrationFile = MigrationFile {
    directory: "20251103072422_add_static_headers",
    bytes: include_bytes!("../migrations/20251103072422_add_static_headers/migration.sql"),
    sha256: "5a06bdfc76f654b56cb4b6e6f7a8cbc8710d8bf8a956fbec315befa047c0851d",
};

pub const ADD_CREDENTIALS_TO_MCP_SERVERS: MigrationFile = MigrationFile {
    directory: "20251104220043_add_credentials_to_mcp_servers",
    bytes: include_bytes!(
        "../migrations/20251104220043_add_credentials_to_mcp_servers/migration.sql"
    ),
    sha256: "d41ba79c0256b803302c6af066885c779a1b1ce5d697347fa8853871d851fc65",
};

pub const ADD_PROJECT_TABLE: MigrationFile = MigrationFile {
    directory: "20251113000000_add_project_table",
    bytes: include_bytes!("../migrations/20251113000000_add_project_table/migration.sql"),
    sha256: "32409a4ca78b502087ab900a2bd32393d3b8de74700666fc9d7386eecaec4e4c",
};

pub const ADD_PROJECT_FIELDS: MigrationFile = MigrationFile {
    directory: "20251113000001_add_project_fields",
    bytes: include_bytes!("../migrations/20251113000001_add_project_fields/migration.sql"),
    sha256: "523da4aa654e59e2ae6f0c41c4523968c25536584602303fadfa88db406844ea",
};

pub const REQUEST_ID_DAILY_TAG_SPEND: MigrationFile = MigrationFile {
    directory: "20251114173537_add_request_id_to_daily_tag_spend",
    bytes: include_bytes!(
        "../migrations/20251114173537_add_request_id_to_daily_tag_spend/migration.sql"
    ),
    sha256: "3ec479bb0a0f34e1f228ba58020fad9dcfed75fee73ec6be86ec00610cc0bdfb",
};

pub const ADD_ORG_USAGE_TABLE: MigrationFile = MigrationFile {
    directory: "20251114180624_Add_org_usage_table",
    bytes: include_bytes!("../migrations/20251114180624_Add_org_usage_table/migration.sql"),
    sha256: "15c266521910eed58078046ec2aacd7fad5bbf3ddb28ae8d7d79ecb0fb4010ba",
};

pub const AGENTS_TABLE: MigrationFile = MigrationFile {
    directory: "20251114182247_agents_table",
    bytes: include_bytes!("../migrations/20251114182247_agents_table/migration.sql"),
    sha256: "68ef6d5784b6d36ca95c15b6f9b20b96f447e10f02665ba86703534d06fdf2b8",
};

pub const ADD_PROMPT_VERSIONING: MigrationFile = MigrationFile {
    directory: "20251119131227_add_prompt_versioning",
    bytes: include_bytes!("../migrations/20251119131227_add_prompt_versioning/migration.sql"),
    sha256: "6053bd301e39dc7d7b2f5c83b21433c30e5e8ec2bfa83d985336738839f22726",
};

pub const ORGANIZATION_ID_SPEND_LOGS: MigrationFile = MigrationFile {
    directory: "20251122125322_Add organization_id to spend logs",
    bytes: include_bytes!(
        "../migrations/20251122125322_Add organization_id to spend logs/migration.sql"
    ),
    sha256: "be594412b4c2cfad28c75e832b6b4f2621391b75db3fb7b88f9f5fa97886bc91",
};

pub const ADD_END_USER_SPEND_TABLE: MigrationFile = MigrationFile {
    directory: "20251204124859_add_end_user_spend_table",
    bytes: include_bytes!("../migrations/20251204124859_add_end_user_spend_table/migration.sql"),
    sha256: "331197ce83fc4cc6c48238c591363da0a5a532af548ec8cddd2d8d6730c2b4a7",
};

pub const ADD_AGENT_PERMISSIONS: MigrationFile = MigrationFile {
    directory: "20251204142718_add_agent_permissions",
    bytes: include_bytes!("../migrations/20251204142718_add_agent_permissions/migration.sql"),
    sha256: "5459cdb798783a9de630bf4d2b1b1ef635f684731c5d9ebfde6874b6ca0850de",
};

pub const ADD_UI_SETTINGS_TABLE: MigrationFile = MigrationFile {
    directory: "20251209112246_add_ui_settings_table",
    bytes: include_bytes!("../migrations/20251209112246_add_ui_settings_table/migration.sql"),
    sha256: "010ec83f8a41f8f746cc866b041bb3407e87367db5e3a84a4c8b3b5ecc57b325",
};

pub const STORAGE_BACKEND_MANAGED_FILES: MigrationFile = MigrationFile {
    directory: "20251210125210_add_storage_backend_to_managed_files",
    bytes: include_bytes!(
        "../migrations/20251210125210_add_storage_backend_to_managed_files/migration.sql"
    ),
    sha256: "6f6ad32b9362509285df9efc255a3490223408dc85100c846a7cfc65365e1121",
};

pub const ADD_DAILY_AGENT_SPEND_TABLE: MigrationFile = MigrationFile {
    directory: "20251210205007_add_daily_agent_spend_table",
    bytes: include_bytes!("../migrations/20251210205007_add_daily_agent_spend_table/migration.sql"),
    sha256: "e51e435dbceb05a9faf3030ad2136d5befdba1b2cd8a0b740c6ea501a75467e8",
};

pub const SCHEMA_SYNC: MigrationFile = MigrationFile {
    directory: "20251211100212_schema_sync",
    bytes: include_bytes!("../migrations/20251211100212_schema_sync/migration.sql"),
    sha256: "b53ebce8addd1ea28efc4b69840bfbece14dce5422418ce99171cfe30493664e",
};

pub const MIGRATION_CATALOG: MigrationCatalog = MigrationCatalog {
    files: &[
        ADD_MCP_NAMESPACED_TOOL_NAME,
        ADD_STDIO_MCP,
        LITELLM_PARAMS_VECTOR_STORES,
        PROMPT_TABLE,
        RENAME_ALIAS_TO_SERVER_NAME,
        DROP_SPEC_VERSION_COLUMN,
        UNNAMED_MIGRATION,
        ADD_ALLOWED_TOOLS_TO_MCP,
        EXTRA_HEADERS_TO_MCP_TABLE,
        ADD_MCP_TOOL_PERMISSIONS,
        ADD_TAG_TABLE,
        ADD_SEARCH_TOOL_TABLE,
        ADD_CACHE_CONFIG_TABLE,
        MANAGED_VECTOR_STORE_INDEX,
        ADD_STATIC_HEADERS,
        ADD_CREDENTIALS_TO_MCP_SERVERS,
        ADD_PROJECT_TABLE,
        ADD_PROJECT_FIELDS,
        REQUEST_ID_DAILY_TAG_SPEND,
        ADD_ORG_USAGE_TABLE,
        AGENTS_TABLE,
        ADD_PROMPT_VERSIONING,
        ORGANIZATION_ID_SPEND_LOGS,
        ADD_END_USER_SPEND_TABLE,
        ADD_AGENT_PERMISSIONS,
        ADD_UI_SETTINGS_TABLE,
        STORAGE_BACKEND_MANAGED_FILES,
        ADD_DAILY_AGENT_SPEND_TABLE,
        SCHEMA_SYNC,
    ],
};

mod tests;
