use crate::schema_baseline::{MigrationCatalog, MigrationFile};

pub const ADD_DAILY_USER_TABLE: MigrationFile = MigrationFile {
    directory: "20250326171002_add_daily_user_table",
    bytes: include_bytes!("../migrations/20250326171002_add_daily_user_table/migration.sql"),
    sha256: "48b046b5dcfe063869b28fd4c47ed4aa553707ccb02a42be276c43ae0d7cd612",
};

pub const ADD_API_REQUESTS_DAILY_USER: MigrationFile = MigrationFile {
    directory: "20250327180120_add_api_requests_to_daily_user_table",
    bytes: include_bytes!(
        "../migrations/20250327180120_add_api_requests_to_daily_user_table/migration.sql"
    ),
    sha256: "47ffc522eec4af95fc48c18d4f92b86a6dd6c755b2afb40dc4585b69ebd109da",
};

pub const NEW_CRON_JOB_TABLE: MigrationFile = MigrationFile {
    directory: "20250329084805_new_cron_job_table",
    bytes: include_bytes!("../migrations/20250329084805_new_cron_job_table/migration.sql"),
    sha256: "050f9d2bf329e99b1c294a5c137d73859dded58324af55696d5c7f51f7e281d4",
};

pub const TRACK_SUCCESS_FAILED_REQUESTS: MigrationFile = MigrationFile {
    directory: "20250331215456_track_success_and_failed_requests_daily_agg_table",
    bytes: include_bytes!(
        "../migrations/20250331215456_track_success_and_failed_requests_daily_agg_table/migration.sql"
    ),
    sha256: "049767abdd55f43520a1c2201a9a99f7356be451bec8c7e52895301fa0e2eb61",
};

pub const ADD_MANAGED_FILE_TABLE: MigrationFile = MigrationFile {
    directory: "20250411215431_add_managed_file_table",
    bytes: include_bytes!("../migrations/20250411215431_add_managed_file_table/migration.sql"),
    sha256: "477ee9e678d0a6f4a3e97898b085a0dfb6ddd24730c28f46e68a700f37ee2d2f",
};

pub const TEAM_MEMBER_PERMISSIONS: MigrationFile = MigrationFile {
    directory: "20250412081753_team_member_permissions",
    bytes: include_bytes!("../migrations/20250412081753_team_member_permissions/migration.sql"),
    sha256: "67b629914bbb8f6fa26c52fac9fd4940a96e5c1371d76dbadc620de66a554244",
};

pub const ADD_CACHE_TOKENS_DAILY_SPEND: MigrationFile = MigrationFile {
    directory: "20250415151647_add_cache_read_write_tokens_daily_spend_transactions",
    bytes: include_bytes!(
        "../migrations/20250415151647_add_cache_read_write_tokens_daily_spend_transactions/migration.sql"
    ),
    sha256: "bc904bdd0befbb6e2fb96b6f4bb7699d1c85518b85cce7f1822524d64d67c3ff",
};

pub const ADD_DAILY_TEAM_TABLE: MigrationFile = MigrationFile {
    directory: "20250415191926_add_daily_team_table",
    bytes: include_bytes!("../migrations/20250415191926_add_daily_team_table/migration.sql"),
    sha256: "f07d74cf601a93592bf6fb0cd732d21a261a23c461b4d64a5eb500b82f38e1c8",
};

pub const ADD_TAG_TABLE_TO_DB: MigrationFile = MigrationFile {
    directory: "20250416115320_add_tag_table_to_db",
    bytes: include_bytes!("../migrations/20250416115320_add_tag_table_to_db/migration.sql"),
    sha256: "7a4a9a98c40803f33f46ece325a233687df1d69402e7536b5e0ae3fb7cdb94d8",
};

pub const DROP_TAG_UNIQUE_CONSTRAINT: MigrationFile = MigrationFile {
    directory: "20250416151339_drop_tag_uniqueness_requirement",
    bytes: include_bytes!(
        "../migrations/20250416151339_drop_tag_uniqueness_requirement/migration.sql"
    ),
    sha256: "e795d3fc96efff0b386425d5d1090319280317dd5954a01614f828a6e020302a",
};

pub const ADD_ALLOWED_ROUTES_TOKEN: MigrationFile = MigrationFile {
    directory: "20250416185146_add_allowed_routes_litellm_verification_token",
    bytes: include_bytes!(
        "../migrations/20250416185146_add_allowed_routes_litellm_verification_token/migration.sql"
    ),
    sha256: "35377cad4aaf835bd319f56811c344d53654ea664216f39c15ddd20434c1b69c",
};

pub const ADD_SESSION_ID: MigrationFile = MigrationFile {
    directory: "20250425182129_add_session_id",
    bytes: include_bytes!("../migrations/20250425182129_add_session_id/migration.sql"),
    sha256: "1cb236b5c18b69b62b58e8c92cb762cb0be309cfdf2bfd935733ba198673d726",
};

pub const ADD_MANAGED_VECTOR_STORES: MigrationFile = MigrationFile {
    directory: "20250430193429_add_managed_vector_stores",
    bytes: include_bytes!("../migrations/20250430193429_add_managed_vector_stores/migration.sql"),
    sha256: "d89c388b45a1367f0ad93543e56bfeaa32a391ff8adf1c18ab74003635446261",
};

pub const ADD_MCP_TABLE_TO_DB: MigrationFile = MigrationFile {
    directory: "20250507161526_add_mcp_table_to_db",
    bytes: include_bytes!("../migrations/20250507161526_add_mcp_table_to_db/migration.sql"),
    sha256: "f03c48cd15a994d4e6727cf4749ad27d52277da0d76561e9d7f50de94e8379b0",
};

pub const ADD_HEALTH_CHECK_MCP_FIELDS: MigrationFile = MigrationFile {
    directory: "20250507161527_add_health_check_fields_to_mcp_servers",
    bytes: include_bytes!(
        "../migrations/20250507161527_add_health_check_fields_to_mcp_servers/migration.sql"
    ),
    sha256: "b0cb512a90b674c5531b2c722048c936efefe834b3050715483d1b9c3c195de2",
};

pub const ADD_MCP_KEY_TEAM_PERMISSION: MigrationFile = MigrationFile {
    directory: "20250507184818_add_mcp_key_team_permission_mgmt",
    bytes: include_bytes!(
        "../migrations/20250507184818_add_mcp_key_team_permission_mgmt/migration.sql"
    ),
    sha256: "3fc693f5dbdbb9968a6d95f2c940a04646b258eb671c70ff01bf4772e27e5979",
};

pub const ADD_STATUS_TO_SPENDLOGS: MigrationFile = MigrationFile {
    directory: "20250508072103_add_status_to_spendlogs",
    bytes: include_bytes!("../migrations/20250508072103_add_status_to_spendlogs/migration.sql"),
    sha256: "ad1052154ae85e7a3beac99764f23ee50feb0eb745a0970935a471d5e730f433",
};

pub const WIDEN_DAILY_SPEND_COUNTERS: MigrationFile = MigrationFile {
    directory: "20250509141545_use_big_int_for_daily_spend_tables",
    bytes: include_bytes!(
        "../migrations/20250509141545_use_big_int_for_daily_spend_tables/migration.sql"
    ),
    sha256: "50f6805ecaf21af5594ff4a739ffb5c87bb96ca3d356bda03f7cd8eb814a47ba",
};

pub const ADD_SPEND_LOGS_SESSION_IDX: MigrationFile = MigrationFile {
    directory: "20250510142544_add_session_id_index_spend_logs",
    bytes: include_bytes!(
        "../migrations/20250510142544_add_session_id_index_spend_logs/migration.sql"
    ),
    sha256: "f718b2a74f2a8529c079242e36330206a17b985d31d36e16fc31861090a74c93",
};

pub const ADD_GUARDRAILS_TABLE: MigrationFile = MigrationFile {
    directory: "20250514142245_add_guardrails_table",
    bytes: include_bytes!("../migrations/20250514142245_add_guardrails_table/migration.sql"),
    sha256: "67fdce0f3f25d350bc661f8f32b747ee7cd3e18dab46a90f72ef7e46024098af",
};

pub const MANAGED_OBJECT_TABLE: MigrationFile = MigrationFile {
    directory: "20250522223020_managed_object_table",
    bytes: include_bytes!("../migrations/20250522223020_managed_object_table/migration.sql"),
    sha256: "9f4099edfb29658b8330afec322d97f2f1233fae81946c51521bbdcde5bf67e0",
};

pub const ALLOW_NULL_ENTITY_ID: MigrationFile = MigrationFile {
    directory: "20250526154401_allow_null_entity_id",
    bytes: include_bytes!("../migrations/20250526154401_allow_null_entity_id/migration.sql"),
    sha256: "3bd0b7fd731a1d2e43aaefc346dca6f3d6e1b29022792422429c4ad4c4d06604",
};

pub const ADD_VECTOR_STORES_PERMISSIONS: MigrationFile = MigrationFile {
    directory: "20250528185438_add_vector_stores_to_object_permissions",
    bytes: include_bytes!(
        "../migrations/20250528185438_add_vector_stores_to_object_permissions/migration.sql"
    ),
    sha256: "55939a32735a821668ea76cf37ad550e0f567225eec6d7595221a9f7445e5854",
};

pub const CASCADE_BUDGET_CHANGES: MigrationFile = MigrationFile {
    directory: "20250603210143_cascade_budget_changes",
    bytes: include_bytes!("../migrations/20250603210143_cascade_budget_changes/migration.sql"),
    sha256: "c2eb9e521552dbb2f7f14652a7f0d66c892589745ef121c1799e06fa5bd53e2d",
};

pub const ADD_HEALTH_CHECK_TABLE: MigrationFile = MigrationFile {
    directory: "20250618225828_add_health_check_table",
    bytes: include_bytes!("../migrations/20250618225828_add_health_check_table/migration.sql"),
    sha256: "9e8723ee10d9ce8c5dcad41da4bcb2f8db2d7cab415eff59eb3ecc38b698d2e9",
};

pub const CASCADE_BUDGET_LOOSEN_FILE: MigrationFile = MigrationFile {
    directory: "20250625145206_cascade_budget_and_loosen_managed_file_json",
    bytes: include_bytes!(
        "../migrations/20250625145206_cascade_budget_and_loosen_managed_file_json/migration.sql"
    ),
    sha256: "4ae034e93702511a6641b53102836f239cfee44393703b4ac0be0a5849f69bde",
};

pub const ADD_STATUS_MANAGED_OBJECT: MigrationFile = MigrationFile {
    directory: "20250625213625_add_status_to_managed_object_table",
    bytes: include_bytes!(
        "../migrations/20250625213625_add_status_to_managed_object_table/migration.sql"
    ),
    sha256: "a397ffbfaba32f88d7d453b28f24e3dc8dc240974926e0456d9eaab06fb34b09",
};

pub const ADD_MCP_INFO_COLUMN: MigrationFile = MigrationFile {
    directory: "20250707212517_add_mcp_info_column_mcp_servers",
    bytes: include_bytes!(
        "../migrations/20250707212517_add_mcp_info_column_mcp_servers/migration.sql"
    ),
    sha256: "820dbb9d88eba37648b5e65c2beedfaa022276fc5a704a8fddafb5e36dcc9038",
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
