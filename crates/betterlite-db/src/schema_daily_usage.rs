use crate::schema_baseline::{MigrationCatalog, MigrationFile};

pub const ADD_DAILY_USER_TABLE: MigrationFile = MigrationFile {
    directory: "20250326171002_add_daily_user_table",
    bytes: include_bytes!("../migrations/20250326171002_add_daily_user_table/migration.sql"),
    sha256: "39f57248e7e0a3e27292b87c6f3e1977df431104fdb1d5ed07db21cc4c46f0d5",
};

pub const ADD_API_REQUESTS_DAILY_USER: MigrationFile = MigrationFile {
    directory: "20250327180120_add_api_requests_to_daily_user_table",
    bytes: include_bytes!(
        "../migrations/20250327180120_add_api_requests_to_daily_user_table/migration.sql"
    ),
    sha256: "d3a42413261b6b6757b6c21e6fbab902985459ddff11119fd5c5226ab6c0ee3f",
};

pub const NEW_CRON_JOB_TABLE: MigrationFile = MigrationFile {
    directory: "20250329084805_new_cron_job_table",
    bytes: include_bytes!("../migrations/20250329084805_new_cron_job_table/migration.sql"),
    sha256: "33aaea20e43d2fa5d4c977ba45394f3993d2bccd72261515538c30a01e87428a",
};

pub const TRACK_SUCCESS_FAILED_REQUESTS: MigrationFile = MigrationFile {
    directory: "20250331215456_track_success_and_failed_requests_daily_agg_table",
    bytes: include_bytes!(
        "../migrations/20250331215456_track_success_and_failed_requests_daily_agg_table/migration.sql"
    ),
    sha256: "c976a865e4122d55d7d62fa56a655c92fc77492cf1f63f2eef7395045a894011",
};

pub const ADD_MANAGED_FILE_TABLE: MigrationFile = MigrationFile {
    directory: "20250411215431_add_managed_file_table",
    bytes: include_bytes!("../migrations/20250411215431_add_managed_file_table/migration.sql"),
    sha256: "afd338ae9848a56717a20b9add048409df7eba2f99518c30ca9b4c1bd9425194",
};

pub const TEAM_MEMBER_PERMISSIONS: MigrationFile = MigrationFile {
    directory: "20250412081753_team_member_permissions",
    bytes: include_bytes!("../migrations/20250412081753_team_member_permissions/migration.sql"),
    sha256: "1a567eea551338642c071d10a3c7a3de50d9f1fa8abf0e35eca5d32b6efeb1f2",
};

pub const ADD_CACHE_TOKENS_DAILY_SPEND: MigrationFile = MigrationFile {
    directory: "20250415151647_add_cache_read_write_tokens_daily_spend_transactions",
    bytes: include_bytes!(
        "../migrations/20250415151647_add_cache_read_write_tokens_daily_spend_transactions/migration.sql"
    ),
    sha256: "0dca3c8ca813ccbe688a85f4e6f8ce60e1b90170faede3b51a669d659a64e423",
};

pub const ADD_DAILY_TEAM_TABLE: MigrationFile = MigrationFile {
    directory: "20250415191926_add_daily_team_table",
    bytes: include_bytes!("../migrations/20250415191926_add_daily_team_table/migration.sql"),
    sha256: "16c625997675d045dce697a644f7785dd37965959d7bf11e0ccd7c62145cb7d7",
};

pub const ADD_TAG_TABLE_TO_DB: MigrationFile = MigrationFile {
    directory: "20250416115320_add_tag_table_to_db",
    bytes: include_bytes!("../migrations/20250416115320_add_tag_table_to_db/migration.sql"),
    sha256: "6b286dda31f0939cb31d28e211cc0ebe7dbd216dc8b5f0f3dac2b3230b0749da",
};

pub const DROP_TAG_UNIQUE_CONSTRAINT: MigrationFile = MigrationFile {
    directory: "20250416151339_drop_tag_uniqueness_requirement",
    bytes: include_bytes!(
        "../migrations/20250416151339_drop_tag_uniqueness_requirement/migration.sql"
    ),
    sha256: "f23c3fa67751922f2c3f94d884ea074818313795aee51ec9e126a2e65f0a1db5",
};

pub const ADD_ALLOWED_ROUTES_TOKEN: MigrationFile = MigrationFile {
    directory: "20250416185146_add_allowed_routes_litellm_verification_token",
    bytes: include_bytes!(
        "../migrations/20250416185146_add_allowed_routes_litellm_verification_token/migration.sql"
    ),
    sha256: "b4adc8f7741f6105836918511c8431d4d78fcd1a0a0570dd3706e691855f7fd8",
};

pub const ADD_SESSION_ID: MigrationFile = MigrationFile {
    directory: "20250425182129_add_session_id",
    bytes: include_bytes!("../migrations/20250425182129_add_session_id/migration.sql"),
    sha256: "ea6a41049e3f1215cd80897323e6aa09665067bd7d356493f1858a11969e5ee2",
};

pub const ADD_MANAGED_VECTOR_STORES: MigrationFile = MigrationFile {
    directory: "20250430193429_add_managed_vector_stores",
    bytes: include_bytes!("../migrations/20250430193429_add_managed_vector_stores/migration.sql"),
    sha256: "1b6ef4077162e63bc735d174f5f9ff629c0382902ef899b0cf16ac337446d290",
};

pub const ADD_MCP_TABLE_TO_DB: MigrationFile = MigrationFile {
    directory: "20250507161526_add_mcp_table_to_db",
    bytes: include_bytes!("../migrations/20250507161526_add_mcp_table_to_db/migration.sql"),
    sha256: "ab157bc28700b74a55876a740b0ffd4f35e214e200bdd5b2000edc933adea15c",
};

pub const ADD_HEALTH_CHECK_MCP_FIELDS: MigrationFile = MigrationFile {
    directory: "20250507161527_add_health_check_fields_to_mcp_servers",
    bytes: include_bytes!(
        "../migrations/20250507161527_add_health_check_fields_to_mcp_servers/migration.sql"
    ),
    sha256: "6a4618a2897815963c539d71c748f2f05c397df4be445f238dde9178f465e340",
};

pub const ADD_MCP_KEY_TEAM_PERMISSION: MigrationFile = MigrationFile {
    directory: "20250507184818_add_mcp_key_team_permission_mgmt",
    bytes: include_bytes!(
        "../migrations/20250507184818_add_mcp_key_team_permission_mgmt/migration.sql"
    ),
    sha256: "26bc2bc48529a1cf1ffeff236103d1c6e468ef7ca6981fbaaaf74bd1ff905d75",
};

pub const ADD_STATUS_TO_SPENDLOGS: MigrationFile = MigrationFile {
    directory: "20250508072103_add_status_to_spendlogs",
    bytes: include_bytes!("../migrations/20250508072103_add_status_to_spendlogs/migration.sql"),
    sha256: "b2926aeb76a7aaf16d1febfaa5ee4c6e12305c06bdc444bbd7fe729ad23c4727",
};

pub const WIDEN_DAILY_SPEND_COUNTERS: MigrationFile = MigrationFile {
    directory: "20250509141545_use_big_int_for_daily_spend_tables",
    bytes: include_bytes!(
        "../migrations/20250509141545_use_big_int_for_daily_spend_tables/migration.sql"
    ),
    sha256: "d1faf058b37ff20cc5cfca5bf7d99cf31fad573ab1cd2b620a5150fd338fd457",
};

pub const ADD_SPEND_LOGS_SESSION_IDX: MigrationFile = MigrationFile {
    directory: "20250510142544_add_session_id_index_spend_logs",
    bytes: include_bytes!(
        "../migrations/20250510142544_add_session_id_index_spend_logs/migration.sql"
    ),
    sha256: "bd56e78f23855ff9b47615f6f291d872162e52959af34209f2b8f970d42024a9",
};

pub const ADD_GUARDRAILS_TABLE: MigrationFile = MigrationFile {
    directory: "20250514142245_add_guardrails_table",
    bytes: include_bytes!("../migrations/20250514142245_add_guardrails_table/migration.sql"),
    sha256: "853f4a10da0b99eaed34e7f42c6f256e70994713f3f4434fa74d2e9fde443840",
};

pub const MANAGED_OBJECT_TABLE: MigrationFile = MigrationFile {
    directory: "20250522223020_managed_object_table",
    bytes: include_bytes!("../migrations/20250522223020_managed_object_table/migration.sql"),
    sha256: "8acbbbc9ad0a8b70985f39eeec5c48fd5803c9a96dad48d0b753d33009d8b507",
};

pub const ALLOW_NULL_ENTITY_ID: MigrationFile = MigrationFile {
    directory: "20250526154401_allow_null_entity_id",
    bytes: include_bytes!("../migrations/20250526154401_allow_null_entity_id/migration.sql"),
    sha256: "93ff1760c1fea073b04fd6c73439e5431bf63d2c0f5fd31c3aa0fd9df29ac51a",
};

pub const ADD_VECTOR_STORES_PERMISSIONS: MigrationFile = MigrationFile {
    directory: "20250528185438_add_vector_stores_to_object_permissions",
    bytes: include_bytes!(
        "../migrations/20250528185438_add_vector_stores_to_object_permissions/migration.sql"
    ),
    sha256: "ead8eea3c222730f2623c06712b0af12ac76f977616f0a71efff9a9ddb95c4b0",
};

pub const CASCADE_BUDGET_CHANGES: MigrationFile = MigrationFile {
    directory: "20250603210143_cascade_budget_changes",
    bytes: include_bytes!("../migrations/20250603210143_cascade_budget_changes/migration.sql"),
    sha256: "cfa6fefdbc789febd28ad7d95f52e27e619ded66c119f9f3bf0182ce01d8541c",
};

pub const ADD_HEALTH_CHECK_TABLE: MigrationFile = MigrationFile {
    directory: "20250618225828_add_health_check_table",
    bytes: include_bytes!("../migrations/20250618225828_add_health_check_table/migration.sql"),
    sha256: "8460d09314969ee7d3bb6ed486007258e25822d716e92fecb9c5c4e65e72d28f",
};

pub const CASCADE_BUDGET_LOOSEN_FILE: MigrationFile = MigrationFile {
    directory: "20250625145206_cascade_budget_and_loosen_managed_file_json",
    bytes: include_bytes!(
        "../migrations/20250625145206_cascade_budget_and_loosen_managed_file_json/migration.sql"
    ),
    sha256: "4f66aacb25e080d5fe0b314adf27a8950995078bbd349f1b35133530256a7e2f",
};

pub const ADD_STATUS_MANAGED_OBJECT: MigrationFile = MigrationFile {
    directory: "20250625213625_add_status_to_managed_object_table",
    bytes: include_bytes!(
        "../migrations/20250625213625_add_status_to_managed_object_table/migration.sql"
    ),
    sha256: "bc01e93d1735db653dad19fa915151819e810aa85824f58b719e2d171708bf08",
};

pub const ADD_MCP_INFO_COLUMN: MigrationFile = MigrationFile {
    directory: "20250707212517_add_mcp_info_column_mcp_servers",
    bytes: include_bytes!(
        "../migrations/20250707212517_add_mcp_info_column_mcp_servers/migration.sql"
    ),
    sha256: "8ce85445c4d7db6a2ad3ea99dc46f7d76ba7a5d77de265286e91d89caae0d8da",
};

pub const MIGRATION_CATALOG: MigrationCatalog = MigrationCatalog {
    files: &[
        ADD_DAILY_USER_TABLE,
        ADD_API_REQUESTS_DAILY_USER,
        NEW_CRON_JOB_TABLE,
        TRACK_SUCCESS_FAILED_REQUESTS,
        ADD_MANAGED_FILE_TABLE,
        TEAM_MEMBER_PERMISSIONS,
        ADD_CACHE_TOKENS_DAILY_SPEND,
        ADD_DAILY_TEAM_TABLE,
        ADD_TAG_TABLE_TO_DB,
        DROP_TAG_UNIQUE_CONSTRAINT,
        ADD_ALLOWED_ROUTES_TOKEN,
        ADD_SESSION_ID,
        ADD_MANAGED_VECTOR_STORES,
        ADD_MCP_TABLE_TO_DB,
        ADD_HEALTH_CHECK_MCP_FIELDS,
        ADD_MCP_KEY_TEAM_PERMISSION,
        ADD_STATUS_TO_SPENDLOGS,
        WIDEN_DAILY_SPEND_COUNTERS,
        ADD_SPEND_LOGS_SESSION_IDX,
        ADD_GUARDRAILS_TABLE,
        MANAGED_OBJECT_TABLE,
        ALLOW_NULL_ENTITY_ID,
        ADD_VECTOR_STORES_PERMISSIONS,
        CASCADE_BUDGET_CHANGES,
        ADD_HEALTH_CHECK_TABLE,
        CASCADE_BUDGET_LOOSEN_FILE,
        ADD_STATUS_MANAGED_OBJECT,
        ADD_MCP_INFO_COLUMN,
    ],
};

mod tests;
