#![cfg(test)]

use crate::schema_baseline::{MigrationCatalog, MigrationFile};

const PINNED_DELETED_KEYS_TEAMS: MigrationFile = MigrationFile {
    directory: "20251219110931_add_deleted_keys_and_deleted_teams_tables",
    bytes: include_bytes!("fixtures/20251219110931_add_deleted_keys_and_deleted_teams_tables.sql"),
    sha256: "4b73b74abadffbf68ef2506300d2c6c939939b15649ac304b676521c29a65888",
};

const PINNED_SCHEMA_UPDATE: MigrationFile = MigrationFile {
    directory: "20251220144550_schema_update",
    bytes: include_bytes!("fixtures/20251220144550_schema_update.sql"),
    sha256: "f39d63a4a4e8ded020521d9f219d1df7b56a65add66ad838b5c0c3ee23ec6591",
};

const PINNED_MCP_METADATA_URLS: MigrationFile = MigrationFile {
    directory: "20260102131258_add_metadata_urls_to_mcp_servers",
    bytes: include_bytes!("fixtures/20260102131258_add_metadata_urls_to_mcp_servers.sql"),
    sha256: "e737c214e6369e38c11e555e292476106edd0b244ff75cfbfbc94141507c1a28",
};

const PINNED_MCP_ALLOW_ALL_KEYS: MigrationFile = MigrationFile {
    directory: "20260105151539_add_allow_all_keys_to_mcp_servers",
    bytes: include_bytes!("fixtures/20260105151539_add_allow_all_keys_to_mcp_servers.sql"),
    sha256: "760590190fb672c360834f9ab213613293b219e04f7e35ae53d686a413a6d868",
};

const PINNED_DAILY_ENDPOINT: MigrationFile = MigrationFile {
    directory: "20260106155622_add_endpoint_to_daily_activity_tables",
    bytes: include_bytes!("fixtures/20260106155622_add_endpoint_to_daily_activity_tables.sql"),
    sha256: "3517dbffaa550d73e596ed11c802e78699c72861d45d61ed4c70c9fd3959d62f",
};

const PINNED_ROUTER_SETTINGS: MigrationFile = MigrationFile {
    directory: "20260107111013_add_router_settings_to_keys_teams",
    bytes: include_bytes!("fixtures/20260107111013_add_router_settings_to_keys_teams.sql"),
    sha256: "a5e4b095c0631bbed46c5b7104b64211368dab8307c22e46fab44933c6bf18de",
};

const PINNED_USER_EMAIL_LOWER_IDX: MigrationFile = MigrationFile {
    directory: "20260108_add_user_email_lower_idx",
    bytes: include_bytes!("fixtures/20260108_add_user_email_lower_idx.sql"),
    sha256: "9abb5570fcdda9d2ec39a645eef6f66d3b9308f2b49005afbdbb8f22b1598aa7",
};

const PINNED_UPDATE_DELETED_ROUTING: MigrationFile = MigrationFile {
    directory: "20260116142756_update_deleted_keys_teams_table_routing_settings",
    bytes: include_bytes!(
        "fixtures/20260116142756_update_deleted_keys_teams_table_routing_settings.sql"
    ),
    sha256: "73c0ba24922dcd5c6f5190e6f398c018fd46d839d6e06664515c834e2545b912",
};

const PINNED_POLICY_TABLES_FIELD: MigrationFile = MigrationFile {
    directory: "20260123131407_add_policy_tables_and_policies_field",
    bytes: include_bytes!("fixtures/20260123131407_add_policy_tables_and_policies_field.sql"),
    sha256: "57159c9d6bbb50b4dc95770c13d440cb48993047c8fd6d2dc695ef57554dea02",
};

const PINNED_VECTOR_STORE_TEAM_USER: MigrationFile = MigrationFile {
    directory: "20260131150814_add_team_user_to_vector_stores",
    bytes: include_bytes!("fixtures/20260131150814_add_team_user_to_vector_stores.sql"),
    sha256: "e4d95f60000cf3dac78e96c50155df471c1789eaab7d71eb548ac38278f441e2",
};

const PINNED_DEPRECATED_TOKEN: MigrationFile = MigrationFile {
    directory: "20260203120000_add_deprecated_verification_token_table",
    bytes: include_bytes!("fixtures/20260203120000_add_deprecated_verification_token_table.sql"),
    sha256: "3dc7d10d308db8101ff1b79a92aff63348f3a686736d4bed672c02c8e8ceb3cf",
};

const PINNED_TEAM_GUARDRAIL_CONFIG: MigrationFile = MigrationFile {
    directory: "20260205091235_allow_team_guardrail_config",
    bytes: include_bytes!("fixtures/20260205091235_allow_team_guardrail_config.sql"),
    sha256: "cf43910b69216944802af825fdf1a3c1085bd21f2ac30b9e79224730246f5aae",
};

const PINNED_TEAM_SOFT_BUDGET: MigrationFile = MigrationFile {
    directory: "20260205144610_add_soft_budget_to_team_table",
    bytes: include_bytes!("fixtures/20260205144610_add_soft_budget_to_team_table.sql"),
    sha256: "49e247bb4aeaca831caf22a0cff33119a366aec31e88630a2425542747748860",
};

const PINNED_MCP_PUBLIC_INTERNET: MigrationFile = MigrationFile {
    directory: "20260207093506_add_available_on_public_internet_to_mcp_servers",
    bytes: include_bytes!(
        "fixtures/20260207093506_add_available_on_public_internet_to_mcp_servers.sql"
    ),
    sha256: "179eef2551a83baeb9a3b40c284af21b886172ff89697094e8c268df5ad68ba1",
};

const PINNED_DELETED_TEAMS_BUDGET: MigrationFile = MigrationFile {
    directory: "20260207110613_add_soft_budget_to_deleted_teams_table",
    bytes: include_bytes!("fixtures/20260207110613_add_soft_budget_to_deleted_teams_table.sql"),
    sha256: "8547050096d31b885a4935db66884594d034c66657df0355f307895807052aea",
};

const PINNED_VERIFICATION_TOKEN_IDX: MigrationFile = MigrationFile {
    directory: "20260209085821_add_verificationtoken_indexes",
    bytes: include_bytes!("fixtures/20260209085821_add_verificationtoken_indexes.sql"),
    sha256: "3cea96b5d541ed36ab8bd027ea3a05094ee235ae9867228aa2fb04c3723fb324",
};

const PINNED_ADJUST_POLICY_TAGS: MigrationFile = MigrationFile {
    directory: "20260212103349_adjust_tags_policy_table",
    bytes: include_bytes!("fixtures/20260212103349_adjust_tags_policy_table.sql"),
    sha256: "ee6a07ff48beb4b06199219103883ce2ae460b137881a35b91ee563fb21738d9",
};

const PINNED_ACCESS_GROUP_TABLE: MigrationFile = MigrationFile {
    directory: "20260212143306_add_access_group_table",
    bytes: include_bytes!("fixtures/20260212143306_add_access_group_table.sql"),
    sha256: "8d69bc93f40c5d200480d899e321e5d1b07f9a7e7ab04df1e58064161740c762",
};

const PINNED_VECTOR_STORE_TABLE: MigrationFile = MigrationFile {
    directory: "20260213105436_add_managed_vector_store_table",
    bytes: include_bytes!("fixtures/20260213105436_add_managed_vector_store_table.sql"),
    sha256: "5ac7e8b6bfb25f1a6d931d545e32929108c76612e73eae2e806ea18058d42ecc",
};

const PINNED_GROUP_MODEL_NAMES: MigrationFile = MigrationFile {
    directory: "20260213170952_access_group_change_to_model_name",
    bytes: include_bytes!("fixtures/20260213170952_access_group_change_to_model_name.sql"),
    sha256: "153e085303ec6c02df5d6fc44a152a853a33b498a252b3c8b1c48056a8da231a",
};

const PINNED_SCHEMA_SYNC: MigrationFile = MigrationFile {
    directory: "20260214094754_schema_sync",
    bytes: include_bytes!("fixtures/20260214094754_schema_sync.sql"),
    sha256: "c95be2b4dcb0820216d2e42015c30ee72a4425bfeb1ccc60dee3c23ecddf61bd",
};

const PINNED_POLICY_PIPELINE: MigrationFile = MigrationFile {
    directory: "20260214163027_add_pipeline_to_policy_table",
    bytes: include_bytes!("fixtures/20260214163027_add_pipeline_to_policy_table.sql"),
    sha256: "9473a420afca0ba8bde0655c98441a7a3a6e7650a41f72e9b68fee066dd16dd9",
};

const PINNED_END_USER_PERMISSIONS: MigrationFile = MigrationFile {
    directory: "20260214185341_object_permissions_for_end_users",
    bytes: include_bytes!("fixtures/20260214185341_object_permissions_for_end_users.sql"),
    sha256: "982686895ff5a4603a92f5e44f706e6644fd0f7451e6bf551bf6fb62bf00a87f",
};

const PINNED_KEY_LAST_ACTIVE: MigrationFile = MigrationFile {
    directory: "20260218231534_add_last_active_to_key_table",
    bytes: include_bytes!("fixtures/20260218231534_add_last_active_to_key_table.sql"),
    sha256: "e70be91b037e49aa8b9af5715858d4c61744543d571719c4c49d4195202ab72a",
};

const PINNED_DELETED_KEY_PROJECT_ID: MigrationFile = MigrationFile {
    directory: "20260219105005_add_project_id_to_deleted_keys",
    bytes: include_bytes!("fixtures/20260219105005_add_project_id_to_deleted_keys.sql"),
    sha256: "1695c1dc949d9859eaed984dbf6e06cb5972b5efafffb83ed567b7385d2dd5bc",
};

pub(super) const PINNED_MIGRATION_CATALOG: MigrationCatalog = MigrationCatalog {
    files: &[
        PINNED_DELETED_KEYS_TEAMS,
        PINNED_SCHEMA_UPDATE,
        PINNED_MCP_METADATA_URLS,
        PINNED_MCP_ALLOW_ALL_KEYS,
        PINNED_DAILY_ENDPOINT,
        PINNED_ROUTER_SETTINGS,
        PINNED_USER_EMAIL_LOWER_IDX,
        PINNED_UPDATE_DELETED_ROUTING,
        PINNED_POLICY_TABLES_FIELD,
        PINNED_VECTOR_STORE_TEAM_USER,
        PINNED_DEPRECATED_TOKEN,
        PINNED_TEAM_GUARDRAIL_CONFIG,
        PINNED_TEAM_SOFT_BUDGET,
        PINNED_MCP_PUBLIC_INTERNET,
        PINNED_DELETED_TEAMS_BUDGET,
        PINNED_VERIFICATION_TOKEN_IDX,
        PINNED_ADJUST_POLICY_TAGS,
        PINNED_ACCESS_GROUP_TABLE,
        PINNED_VECTOR_STORE_TABLE,
        PINNED_GROUP_MODEL_NAMES,
        PINNED_SCHEMA_SYNC,
        PINNED_POLICY_PIPELINE,
        PINNED_END_USER_PERMISSIONS,
        PINNED_KEY_LAST_ACTIVE,
        PINNED_DELETED_KEY_PROJECT_ID,
    ],
};
