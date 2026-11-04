use crate::schema_baseline::{MigrationCatalog, MigrationFile};

pub mod fixtures;

pub const ADD_MCP_NAMESPACED_TOOL_NAME: MigrationFile = MigrationFile {
    directory: "20250707230009_add_mcp_namespaced_tool_name",
    bytes: include_bytes!(
        "../migrations/20250707230009_add_mcp_namespaced_tool_name/migration.sql"
    ),
    sha256: "f345c94a2d1ba5250b8185f86068dc29c8cb1535f13cf5b670dc25f5fb03d2e6",
};

pub const ADD_STDIO_MCP: MigrationFile = MigrationFile {
    directory: "20250711220620_add_stdio_mcp",
    bytes: include_bytes!("../migrations/20250711220620_add_stdio_mcp/migration.sql"),
    sha256: "b1d0f71ed415833ebf31efbad070bc73e2fafce59ec148d3087071848d5282b9",
};

pub const LITELLM_PARAMS_VECTOR_STORES: MigrationFile = MigrationFile {
    directory: "20250718125714_add_litellm_params_to_vector_stores",
    bytes: include_bytes!(
        "../migrations/20250718125714_add_litellm_params_to_vector_stores/migration.sql"
    ),
    sha256: "11655e562e501337d5b19c56e7d2785f9f3964c16d0bda4f701f9c26c4bedc8e",
};

pub const PROMPT_TABLE: MigrationFile = MigrationFile {
    directory: "20250802162330_prompt_table",
    bytes: include_bytes!("../migrations/20250802162330_prompt_table/migration.sql"),
    sha256: "bf163aaf3bcb043f81cdaba361207009399bc74e64417b30289f241206fd5dfd",
};

pub const RENAME_ALIAS_TO_SERVER_NAME: MigrationFile = MigrationFile {
    directory: "20250806095134_rename_alias_to_server_name_mcp_table",
    bytes: include_bytes!(
        "../migrations/20250806095134_rename_alias_to_server_name_mcp_table/migration.sql"
    ),
    sha256: "0607b76d63bdea31615737631d7e6c0fb4b308b9ebad832affa8a5cc914043ac",
};

pub const DROP_SPEC_VERSION_COLUMN: MigrationFile = MigrationFile {
    directory: "20250918083359_drop_spec_version_column_from_mcp_table",
    bytes: include_bytes!(
        "../migrations/20250918083359_drop_spec_version_column_from_mcp_table/migration.sql"
    ),
    sha256: "9bda666c0d3ce21746477a5a28c2b35a715579e9c55460c50dbc334d790aff1b",
};

pub const UNNAMED_MIGRATION: MigrationFile = MigrationFile {
    directory: "20250926194702_unnamed_migration",
    bytes: include_bytes!("../migrations/20250926194702_unnamed_migration/migration.sql"),
    sha256: "425b6e795967c9f494c5a2d612d0d0d3203d263b843727941ce3a262064c398a",
};

pub const ADD_ALLOWED_TOOLS_TO_MCP: MigrationFile = MigrationFile {
    directory: "20251003165142_add_allowed_tools_to_mcp",
    bytes: include_bytes!("../migrations/20251003165142_add_allowed_tools_to_mcp/migration.sql"),
    sha256: "4f9c1595a2d6bddf693ec845058f9413aeffc8a6ea8b46b53566a36b6d93f1e2",
};

pub const EXTRA_HEADERS_TO_MCP_TABLE: MigrationFile = MigrationFile {
    directory: "20251003190954_extra_headers_to_mcp_table",
    bytes: include_bytes!("../migrations/20251003190954_extra_headers_to_mcp_table/migration.sql"),
    sha256: "fe1ae32b28502546173d9345e4d55d05f9644f87750ba9e0a1b28c35c9291f51",
};

pub const ADD_MCP_TOOL_PERMISSIONS: MigrationFile = MigrationFile {
    directory: "20251006143948_add_mcp_tool_permissions",
    bytes: include_bytes!("../migrations/20251006143948_add_mcp_tool_permissions/migration.sql"),
    sha256: "6562a695c0bc052bfbb76da2ee4db808d6070e0c9e421d54ddf69bf943ec922d",
};

pub const ADD_TAG_TABLE: MigrationFile = MigrationFile {
    directory: "20251011084309_add_tag_table",
    bytes: include_bytes!("../migrations/20251011084309_add_tag_table/migration.sql"),
    sha256: "aa1d44f4a4762cb046d6c1103fcbcb9bd43625188e864d76da236190289194b4",
};

pub const ADD_SEARCH_TOOL_TABLE: MigrationFile = MigrationFile {
    directory: "20251023141814_add_search_tool_table",
    bytes: include_bytes!("../migrations/20251023141814_add_search_tool_table/migration.sql"),
    sha256: "663483c4470c0ede72f80da33da2361d94bba8700387d748960dc20ea7785ccb",
};

pub const ADD_CACHE_CONFIG_TABLE: MigrationFile = MigrationFile {
    directory: "20251031181430_add_cache_config_table",
    bytes: include_bytes!("../migrations/20251031181430_add_cache_config_table/migration.sql"),
    sha256: "2f4cd52ea467195bd838bd8fc9df0ae1c443db0fe4985fa1a83a7032f9b36018",
};

pub const MANAGED_VECTOR_STORE_INDEX: MigrationFile = MigrationFile {
    directory: "20251101131415_add_managed_vector_store_index_table",
    bytes: include_bytes!(
        "../migrations/20251101131415_add_managed_vector_store_index_table/migration.sql"
    ),
    sha256: "95564113904a3f141e4470aa3dfc1f559e3d23a8a3f64e9986833d725ea83b28",
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
    sha256: "f9ca207f007c9d80c433f918d6d432598614df5dc89ec7fc8d9effe5ebf23744",
};

pub const ADD_PROJECT_FIELDS: MigrationFile = MigrationFile {
    directory: "20251113000001_add_project_fields",
    bytes: include_bytes!("../migrations/20251113000001_add_project_fields/migration.sql"),
    sha256: "bb9657c76de699e4527516929492dbb173c2d800b242a5db30c1ff803f56a0b1",
};

pub const REQUEST_ID_DAILY_TAG_SPEND: MigrationFile = MigrationFile {
    directory: "20251114173537_add_request_id_to_daily_tag_spend",
    bytes: include_bytes!(
        "../migrations/20251114173537_add_request_id_to_daily_tag_spend/migration.sql"
    ),
    sha256: "756a19475ebee069e8847fa65f3210ba7bb5b816929a477588c67dfa8adf743f",
};

pub const ADD_ORG_USAGE_TABLE: MigrationFile = MigrationFile {
    directory: "20251114180624_Add_org_usage_table",
    bytes: include_bytes!("../migrations/20251114180624_Add_org_usage_table/migration.sql"),
    sha256: "83ba27846dec4011f8b17df50b47cfb9200f6198c5398a47bf2c0be32fffd430",
};

pub const AGENTS_TABLE: MigrationFile = MigrationFile {
    directory: "20251114182247_agents_table",
    bytes: include_bytes!("../migrations/20251114182247_agents_table/migration.sql"),
    sha256: "04abaf00e118987d0a9de36954f2fddc97c456e4af4f0e203b5141427743f33c",
};

pub const ADD_PROMPT_VERSIONING: MigrationFile = MigrationFile {
    directory: "20251119131227_add_prompt_versioning",
    bytes: include_bytes!("../migrations/20251119131227_add_prompt_versioning/migration.sql"),
    sha256: "9796ddf9bc6994b507ff9f6a9e460a284230ab3f4dde4cd1a9224f21f08e19c5",
};

pub const ORGANIZATION_ID_SPEND_LOGS: MigrationFile = MigrationFile {
    directory: "20251122125322_Add organization_id to spend logs",
    bytes: include_bytes!(
        "../migrations/20251122125322_Add organization_id to spend logs/migration.sql"
    ),
    sha256: "d2723db3432cddfb6f6e0cf85b14297fe0e2157fc4b8dd1e2f7819a740cccc36",
};

pub const ADD_END_USER_SPEND_TABLE: MigrationFile = MigrationFile {
    directory: "20251204124859_add_end_user_spend_table",
    bytes: include_bytes!("../migrations/20251204124859_add_end_user_spend_table/migration.sql"),
    sha256: "bc1914aefc7203fdcaadb7b70ae513b2e75bd72b268f5fde8e144caabb3a1b45",
};

pub const ADD_AGENT_PERMISSIONS: MigrationFile = MigrationFile {
    directory: "20251204142718_add_agent_permissions",
    bytes: include_bytes!("../migrations/20251204142718_add_agent_permissions/migration.sql"),
    sha256: "86038393c8ebaf7bb60df05dfa7024a17fe8c03aa7fac858beaaf8f07ec753bc",
};

pub const ADD_UI_SETTINGS_TABLE: MigrationFile = MigrationFile {
    directory: "20251209112246_add_ui_settings_table",
    bytes: include_bytes!("../migrations/20251209112246_add_ui_settings_table/migration.sql"),
    sha256: "3ad6c3f157817a4b3d37832d0e0cb78455a50466315c20e6462f07ccd166f882",
};

pub const STORAGE_BACKEND_MANAGED_FILES: MigrationFile = MigrationFile {
    directory: "20251210125210_add_storage_backend_to_managed_files",
    bytes: include_bytes!(
        "../migrations/20251210125210_add_storage_backend_to_managed_files/migration.sql"
    ),
    sha256: "2b23aa2aee91f35133eb0e38e413c5979df9ff9949d57ac70550a5a17ad1a53f",
};

pub const ADD_DAILY_AGENT_SPEND_TABLE: MigrationFile = MigrationFile {
    directory: "20251210205007_add_daily_agent_spend_table",
    bytes: include_bytes!("../migrations/20251210205007_add_daily_agent_spend_table/migration.sql"),
    sha256: "43177e6394f79b21baa869588551f62b41349536239891113d3016eae7552353",
};

pub const SCHEMA_SYNC: MigrationFile = MigrationFile {
    directory: "20251211100212_schema_sync",
    bytes: include_bytes!("../migrations/20251211100212_schema_sync/migration.sql"),
    sha256: "375bcf8045e6cacdaeeca5e120516aa9bd57743e926f48cad66dbaeb09f6ad57",
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
