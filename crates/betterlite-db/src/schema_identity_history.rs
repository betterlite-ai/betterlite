use crate::schema_baseline::{MigrationCatalog, MigrationFile};

pub mod fixtures;

pub const ADD_DELETED_KEYS_TEAMS_TABLES: MigrationFile = MigrationFile {
    directory: "20251219110931_add_deleted_keys_and_deleted_teams_tables",
    bytes: include_bytes!(
        "../migrations/20251219110931_add_deleted_keys_and_deleted_teams_tables/migration.sql"
    ),
    sha256: "4b73b74abadffbf68ef2506300d2c6c939939b15649ac304b676521c29a65888",
};

pub const SCHEMA_UPDATE: MigrationFile = MigrationFile {
    directory: "20251220144550_schema_update",
    bytes: include_bytes!("../migrations/20251220144550_schema_update/migration.sql"),
    sha256: "f39d63a4a4e8ded020521d9f219d1df7b56a65add66ad838b5c0c3ee23ec6591",
};

pub const ADD_METADATA_URLS_TO_MCP: MigrationFile = MigrationFile {
    directory: "20260102131258_add_metadata_urls_to_mcp_servers",
    bytes: include_bytes!(
        "../migrations/20260102131258_add_metadata_urls_to_mcp_servers/migration.sql"
    ),
    sha256: "e737c214e6369e38c11e555e292476106edd0b244ff75cfbfbc94141507c1a28",
};

pub const ADD_ALLOW_ALL_KEYS_TO_MCP: MigrationFile = MigrationFile {
    directory: "20260105151539_add_allow_all_keys_to_mcp_servers",
    bytes: include_bytes!(
        "../migrations/20260105151539_add_allow_all_keys_to_mcp_servers/migration.sql"
    ),
    sha256: "760590190fb672c360834f9ab213613293b219e04f7e35ae53d686a413a6d868",
};

pub const ADD_ENDPOINT_TO_DAILY_ACTIVITY: MigrationFile = MigrationFile {
    directory: "20260106155622_add_endpoint_to_daily_activity_tables",
    bytes: include_bytes!(
        "../migrations/20260106155622_add_endpoint_to_daily_activity_tables/migration.sql"
    ),
    sha256: "3517dbffaa550d73e596ed11c802e78699c72861d45d61ed4c70c9fd3959d62f",
};

pub const ADD_ROUTER_SETTINGS_KEYS_TEAMS: MigrationFile = MigrationFile {
    directory: "20260107111013_add_router_settings_to_keys_teams",
    bytes: include_bytes!(
        "../migrations/20260107111013_add_router_settings_to_keys_teams/migration.sql"
    ),
    sha256: "a5e4b095c0631bbed46c5b7104b64211368dab8307c22e46fab44933c6bf18de",
};

pub const ADD_USER_EMAIL_LOWER_IDX: MigrationFile = MigrationFile {
    directory: "20260108_add_user_email_lower_idx",
    bytes: include_bytes!("../migrations/20260108_add_user_email_lower_idx/migration.sql"),
    sha256: "9abb5570fcdda9d2ec39a645eef6f66d3b9308f2b49005afbdbb8f22b1598aa7",
};

pub const UPDATE_DELETED_ROUTING: MigrationFile = MigrationFile {
    directory: "20260116142756_update_deleted_keys_teams_table_routing_settings",
    bytes: include_bytes!(
        "../migrations/20260116142756_update_deleted_keys_teams_table_routing_settings/migration.sql"
    ),
    sha256: "73c0ba24922dcd5c6f5190e6f398c018fd46d839d6e06664515c834e2545b912",
};

pub const ADD_POLICY_TABLES_AND_FIELD: MigrationFile = MigrationFile {
    directory: "20260123131407_add_policy_tables_and_policies_field",
    bytes: include_bytes!(
        "../migrations/20260123131407_add_policy_tables_and_policies_field/migration.sql"
    ),
    sha256: "57159c9d6bbb50b4dc95770c13d440cb48993047c8fd6d2dc695ef57554dea02",
};

pub const ADD_TEAM_USER_TO_VECTOR_STORES: MigrationFile = MigrationFile {
    directory: "20260131150814_add_team_user_to_vector_stores",
    bytes: include_bytes!(
        "../migrations/20260131150814_add_team_user_to_vector_stores/migration.sql"
    ),
    sha256: "e4d95f60000cf3dac78e96c50155df471c1789eaab7d71eb548ac38278f441e2",
};

pub const ADD_DEPRECATED_TOKEN_TABLE: MigrationFile = MigrationFile {
    directory: "20260203120000_add_deprecated_verification_token_table",
    bytes: include_bytes!(
        "../migrations/20260203120000_add_deprecated_verification_token_table/migration.sql"
    ),
    sha256: "3dc7d10d308db8101ff1b79a92aff63348f3a686736d4bed672c02c8e8ceb3cf",
};

pub const ALLOW_TEAM_GUARDRAIL_CONFIG: MigrationFile = MigrationFile {
    directory: "20260205091235_allow_team_guardrail_config",
    bytes: include_bytes!("../migrations/20260205091235_allow_team_guardrail_config/migration.sql"),
    sha256: "cf43910b69216944802af825fdf1a3c1085bd21f2ac30b9e79224730246f5aae",
};

pub const ADD_SOFT_BUDGET_TO_TEAM_TABLE: MigrationFile = MigrationFile {
    directory: "20260205144610_add_soft_budget_to_team_table",
    bytes: include_bytes!(
        "../migrations/20260205144610_add_soft_budget_to_team_table/migration.sql"
    ),
    sha256: "49e247bb4aeaca831caf22a0cff33119a366aec31e88630a2425542747748860",
};

pub const ADD_PUBLIC_INTERNET_TO_MCP: MigrationFile = MigrationFile {
    directory: "20260207093506_add_available_on_public_internet_to_mcp_servers",
    bytes: include_bytes!(
        "../migrations/20260207093506_add_available_on_public_internet_to_mcp_servers/migration.sql"
    ),
    sha256: "179eef2551a83baeb9a3b40c284af21b886172ff89697094e8c268df5ad68ba1",
};

pub const ADD_SOFT_BUDGET_DELETED_TEAMS: MigrationFile = MigrationFile {
    directory: "20260207110613_add_soft_budget_to_deleted_teams_table",
    bytes: include_bytes!(
        "../migrations/20260207110613_add_soft_budget_to_deleted_teams_table/migration.sql"
    ),
    sha256: "8547050096d31b885a4935db66884594d034c66657df0355f307895807052aea",
};

pub const ADD_VERIFICATIONTOKEN_INDEXES: MigrationFile = MigrationFile {
    directory: "20260209085821_add_verificationtoken_indexes",
    bytes: include_bytes!(
        "../migrations/20260209085821_add_verificationtoken_indexes/migration.sql"
    ),
    sha256: "3cea96b5d541ed36ab8bd027ea3a05094ee235ae9867228aa2fb04c3723fb324",
};

pub const ADJUST_TAGS_POLICY_TABLE: MigrationFile = MigrationFile {
    directory: "20260212103349_adjust_tags_policy_table",
    bytes: include_bytes!("../migrations/20260212103349_adjust_tags_policy_table/migration.sql"),
    sha256: "ee6a07ff48beb4b06199219103883ce2ae460b137881a35b91ee563fb21738d9",
};

pub const ADD_ACCESS_GROUP_TABLE: MigrationFile = MigrationFile {
    directory: "20260212143306_add_access_group_table",
    bytes: include_bytes!("../migrations/20260212143306_add_access_group_table/migration.sql"),
    sha256: "8d69bc93f40c5d200480d899e321e5d1b07f9a7e7ab04df1e58064161740c762",
};

pub const ADD_MANAGED_VECTOR_STORE_TABLE: MigrationFile = MigrationFile {
    directory: "20260213105436_add_managed_vector_store_table",
    bytes: include_bytes!(
        "../migrations/20260213105436_add_managed_vector_store_table/migration.sql"
    ),
    sha256: "5ac7e8b6bfb25f1a6d931d545e32929108c76612e73eae2e806ea18058d42ecc",
};

pub const ACCESS_GROUP_TO_MODEL_NAMES: MigrationFile = MigrationFile {
    directory: "20260213170952_access_group_change_to_model_name",
    bytes: include_bytes!(
        "../migrations/20260213170952_access_group_change_to_model_name/migration.sql"
    ),
    sha256: "153e085303ec6c02df5d6fc44a152a853a33b498a252b3c8b1c48056a8da231a",
};

pub const SCHEMA_SYNC: MigrationFile = MigrationFile {
    directory: "20260214094754_schema_sync",
    bytes: include_bytes!("../migrations/20260214094754_schema_sync/migration.sql"),
    sha256: "c95be2b4dcb0820216d2e42015c30ee72a4425bfeb1ccc60dee3c23ecddf61bd",
};

pub const ADD_PIPELINE_TO_POLICY_TABLE: MigrationFile = MigrationFile {
    directory: "20260214163027_add_pipeline_to_policy_table",
    bytes: include_bytes!(
        "../migrations/20260214163027_add_pipeline_to_policy_table/migration.sql"
    ),
    sha256: "9473a420afca0ba8bde0655c98441a7a3a6e7650a41f72e9b68fee066dd16dd9",
};

pub const OBJECT_PERMISSIONS_END_USERS: MigrationFile = MigrationFile {
    directory: "20260214185341_object_permissions_for_end_users",
    bytes: include_bytes!(
        "../migrations/20260214185341_object_permissions_for_end_users/migration.sql"
    ),
    sha256: "982686895ff5a4603a92f5e44f706e6644fd0f7451e6bf551bf6fb62bf00a87f",
};

pub const ADD_LAST_ACTIVE_TO_KEY_TABLE: MigrationFile = MigrationFile {
    directory: "20260218231534_add_last_active_to_key_table",
    bytes: include_bytes!(
        "../migrations/20260218231534_add_last_active_to_key_table/migration.sql"
    ),
    sha256: "e70be91b037e49aa8b9af5715858d4c61744543d571719c4c49d4195202ab72a",
};

pub const ADD_PROJECT_ID_TO_DELETED_KEYS: MigrationFile = MigrationFile {
    directory: "20260219105005_add_project_id_to_deleted_keys",
    bytes: include_bytes!(
        "../migrations/20260219105005_add_project_id_to_deleted_keys/migration.sql"
    ),
    sha256: "1695c1dc949d9859eaed984dbf6e06cb5972b5efafffb83ed567b7385d2dd5bc",
};

pub const MIGRATION_CATALOG: MigrationCatalog = MigrationCatalog {
    files: &[
        ADD_DELETED_KEYS_TEAMS_TABLES,
        SCHEMA_UPDATE,
        ADD_METADATA_URLS_TO_MCP,
        ADD_ALLOW_ALL_KEYS_TO_MCP,
        ADD_ENDPOINT_TO_DAILY_ACTIVITY,
        ADD_ROUTER_SETTINGS_KEYS_TEAMS,
        ADD_USER_EMAIL_LOWER_IDX,
        UPDATE_DELETED_ROUTING,
        ADD_POLICY_TABLES_AND_FIELD,
        ADD_TEAM_USER_TO_VECTOR_STORES,
        ADD_DEPRECATED_TOKEN_TABLE,
        ALLOW_TEAM_GUARDRAIL_CONFIG,
        ADD_SOFT_BUDGET_TO_TEAM_TABLE,
        ADD_PUBLIC_INTERNET_TO_MCP,
        ADD_SOFT_BUDGET_DELETED_TEAMS,
        ADD_VERIFICATIONTOKEN_INDEXES,
        ADJUST_TAGS_POLICY_TABLE,
        ADD_ACCESS_GROUP_TABLE,
        ADD_MANAGED_VECTOR_STORE_TABLE,
        ACCESS_GROUP_TO_MODEL_NAMES,
        SCHEMA_SYNC,
        ADD_PIPELINE_TO_POLICY_TABLE,
        OBJECT_PERMISSIONS_END_USERS,
        ADD_LAST_ACTIVE_TO_KEY_TABLE,
        ADD_PROJECT_ID_TO_DELETED_KEYS,
    ],
};

mod tests;
