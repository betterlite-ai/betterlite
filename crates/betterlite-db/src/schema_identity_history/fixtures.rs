#![cfg(test)]

use crate::schema_baseline::{MigrationCatalog, MigrationFile};

const PINNED_DELETED_KEYS_TEAMS: MigrationFile = MigrationFile {
    directory: "20251219110931_add_deleted_keys_and_deleted_teams_tables",
    bytes: include_bytes!("fixtures/20251219110931_add_deleted_keys_and_deleted_teams_tables.sql"),
    sha256: "3b3de59c965b5fec61886f09fa0c3e406c1d1194b8674353ea4b7baac3cba3af",
};

const PINNED_SCHEMA_UPDATE: MigrationFile = MigrationFile {
    directory: "20251220144550_schema_update",
    bytes: include_bytes!("fixtures/20251220144550_schema_update.sql"),
    sha256: "8e219b15b4e476b22c1716992fd513152039de1c5951a0ac694d1ffba26111b3",
};

const PINNED_MCP_METADATA_URLS: MigrationFile = MigrationFile {
    directory: "20260102131258_add_metadata_urls_to_mcp_servers",
    bytes: include_bytes!("fixtures/20260102131258_add_metadata_urls_to_mcp_servers.sql"),
    sha256: "eee02bb2a33b6bdf324081d9006f8da5f1298a0c7f8fa4ae52391d4b3fdd2d2e",
};

const PINNED_MCP_ALLOW_ALL_KEYS: MigrationFile = MigrationFile {
    directory: "20260105151539_add_allow_all_keys_to_mcp_servers",
    bytes: include_bytes!("fixtures/20260105151539_add_allow_all_keys_to_mcp_servers.sql"),
    sha256: "b28ffa7014e196d4b1a8e7d52371be91d5633a6b661f0e5e6346b3fc18fd6456",
};

const PINNED_DAILY_ENDPOINT: MigrationFile = MigrationFile {
    directory: "20260106155622_add_endpoint_to_daily_activity_tables",
    bytes: include_bytes!("fixtures/20260106155622_add_endpoint_to_daily_activity_tables.sql"),
    sha256: "ccfeb894b1fab623593531f189dcd0c098352f9c65c407cb70d2ea9a6086a656",
};

const PINNED_ROUTER_SETTINGS: MigrationFile = MigrationFile {
    directory: "20260107111013_add_router_settings_to_keys_teams",
    bytes: include_bytes!("fixtures/20260107111013_add_router_settings_to_keys_teams.sql"),
    sha256: "9aebf31dd752cf36c8f084d00053ff83af52bf45845549cbb9cb901147d8f2d5",
};

const PINNED_USER_EMAIL_LOWER_IDX: MigrationFile = MigrationFile {
    directory: "20260108_add_user_email_lower_idx",
    bytes: include_bytes!("fixtures/20260108_add_user_email_lower_idx.sql"),
    sha256: "3cb5404a0f325ccf199bb63b283e4da752d97c5012c736cb9cd00755f8ebae03",
};

const PINNED_UPDATE_DELETED_ROUTING: MigrationFile = MigrationFile {
    directory: "20260116142756_update_deleted_keys_teams_table_routing_settings",
    bytes: include_bytes!(
        "fixtures/20260116142756_update_deleted_keys_teams_table_routing_settings.sql"
    ),
    sha256: "b73e82dcb5dd5607c3635a651f7fbc7d7b9d943504e28e4ae58384d3c970706f",
};

const PINNED_POLICY_TABLES_FIELD: MigrationFile = MigrationFile {
    directory: "20260123131407_add_policy_tables_and_policies_field",
    bytes: include_bytes!("fixtures/20260123131407_add_policy_tables_and_policies_field.sql"),
    sha256: "45714f4a2e87eb33b7ef53a5320235b5cafc8a0cc307b00743e74dae6719ea67",
};

const PINNED_VECTOR_STORE_TEAM_USER: MigrationFile = MigrationFile {
    directory: "20260131150814_add_team_user_to_vector_stores",
    bytes: include_bytes!("fixtures/20260131150814_add_team_user_to_vector_stores.sql"),
    sha256: "547773d84d26f9f215bd7d2674ed404811e541358755220bb716c0b97c81fe86",
};

const PINNED_DEPRECATED_TOKEN: MigrationFile = MigrationFile {
    directory: "20260203120000_add_deprecated_verification_token_table",
    bytes: include_bytes!("fixtures/20260203120000_add_deprecated_verification_token_table.sql"),
    sha256: "3dc7d10d308db8101ff1b79a92aff63348f3a686736d4bed672c02c8e8ceb3cf",
};

const PINNED_TEAM_GUARDRAIL_CONFIG: MigrationFile = MigrationFile {
    directory: "20260205091235_allow_team_guardrail_config",
    bytes: include_bytes!("fixtures/20260205091235_allow_team_guardrail_config.sql"),
    sha256: "1d364b454c528c4662780cc4a0ddb12f4c18e82b5eebc83a72fd4429ff367aa1",
};

const PINNED_TEAM_SOFT_BUDGET: MigrationFile = MigrationFile {
    directory: "20260205144610_add_soft_budget_to_team_table",
    bytes: include_bytes!("fixtures/20260205144610_add_soft_budget_to_team_table.sql"),
    sha256: "f1f3c7982ff5ff7f55b064d247cc39ce39e882c7d02abf621669e1b76e437442",
};

const PINNED_MCP_PUBLIC_INTERNET: MigrationFile = MigrationFile {
    directory: "20260207093506_add_available_on_public_internet_to_mcp_servers",
    bytes: include_bytes!(
        "fixtures/20260207093506_add_available_on_public_internet_to_mcp_servers.sql"
    ),
    sha256: "bdb5f1a12ad5eb20c962d03bfd05d6aee6fce92cfd50972d61864856753b0e3a",
};

const PINNED_DELETED_TEAMS_BUDGET: MigrationFile = MigrationFile {
    directory: "20260207110613_add_soft_budget_to_deleted_teams_table",
    bytes: include_bytes!("fixtures/20260207110613_add_soft_budget_to_deleted_teams_table.sql"),
    sha256: "f7c32024cf470b46700be73100dc1e08010942989e908ceb12a5b0e72f95fb5f",
};

const PINNED_VERIFICATION_TOKEN_IDX: MigrationFile = MigrationFile {
    directory: "20260209085821_add_verificationtoken_indexes",
    bytes: include_bytes!("fixtures/20260209085821_add_verificationtoken_indexes.sql"),
    sha256: "3cea96b5d541ed36ab8bd027ea3a05094ee235ae9867228aa2fb04c3723fb324",
};

const PINNED_ADJUST_POLICY_TAGS: MigrationFile = MigrationFile {
    directory: "20260212103349_adjust_tags_policy_table",
    bytes: include_bytes!("fixtures/20260212103349_adjust_tags_policy_table.sql"),
    sha256: "a782f25b1d9fef4048b24e768f076cabee428060151761cbb9331efa78686ee7",
};

const PINNED_ACCESS_GROUP_TABLE: MigrationFile = MigrationFile {
    directory: "20260212143306_add_access_group_table",
    bytes: include_bytes!("fixtures/20260212143306_add_access_group_table.sql"),
    sha256: "c245f807fcfd72bf80d438f1459fa60c9d4fb445990b154b5ed1bb0fa9ba1872",
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
    sha256: "7269a6d0c0734aced27e3cbcc5841fce76f2f9990b2d44dbcba886f0188e7171",
};

const PINNED_POLICY_PIPELINE: MigrationFile = MigrationFile {
    directory: "20260214163027_add_pipeline_to_policy_table",
    bytes: include_bytes!("fixtures/20260214163027_add_pipeline_to_policy_table.sql"),
    sha256: "ec5ae74d2da8fcc40a42d7f48b2e4ad951677f4e4988a5d52af3665ad54b5e72",
};

const PINNED_END_USER_PERMISSIONS: MigrationFile = MigrationFile {
    directory: "20260214185341_object_permissions_for_end_users",
    bytes: include_bytes!("fixtures/20260214185341_object_permissions_for_end_users.sql"),
    sha256: "be60adf5a2b5cee92eed4e17e2d213d0a12c1debd55a637e7ba05e3a234bb2b3",
};

const PINNED_KEY_LAST_ACTIVE: MigrationFile = MigrationFile {
    directory: "20260218231534_add_last_active_to_key_table",
    bytes: include_bytes!("fixtures/20260218231534_add_last_active_to_key_table.sql"),
    sha256: "7f63ab0118b1e6582eb284dd957c708ff77efe0463219f895a2dee48f0beb6e3",
};

const PINNED_DELETED_KEY_PROJECT_ID: MigrationFile = MigrationFile {
    directory: "20260219105005_add_project_id_to_deleted_keys",
    bytes: include_bytes!("fixtures/20260219105005_add_project_id_to_deleted_keys.sql"),
    sha256: "a8a34c4f3f2e5db16ee907152ebf836f5a63aea777798a66069c8af7997869a4",
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
