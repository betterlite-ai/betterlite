use crate::schema_baseline::MigrationFile;

pub(super) const ADD_SHADOW_EVAL: MigrationFile = MigrationFile {
    directory: "20260811172448_add_shadow_eval",
    bytes: include_bytes!("../../migrations/20260811172448_add_shadow_eval/migration.sql"),
    sha256: "03f670e91b8586922d1a6e8460274697eb73bb365ecdf26eea0727b16b8ba05a",
};

pub(super) const ADD_SHADOW_EVAL_DIRECTION: MigrationFile = MigrationFile {
    directory: "20260813180408_add_shadow_eval_direction",
    bytes: include_bytes!(
        "../../migrations/20260813180408_add_shadow_eval_direction/migration.sql"
    ),
    sha256: "cca1fa5a90e74f31902714b7ef8c147712b6c889f82effb6491a226abbc34f9b",
};

pub(super) const ADD_PROXY_WORKER_HEARTBEAT: MigrationFile = MigrationFile {
    directory: "20260814000000_add_proxy_worker_heartbeat",
    bytes: include_bytes!(
        "../../migrations/20260814000000_add_proxy_worker_heartbeat/migration.sql"
    ),
    sha256: "7b042b28a4b8bfa74ad9e439b2aca4c85ab6e0de707a5f115d8caae287aba059",
};

pub(super) const SHADOW_EVAL_MULTI_KEY: MigrationFile = MigrationFile {
    directory: "20260817000000_shadow_eval_multi_key",
    bytes: include_bytes!("../../migrations/20260817000000_shadow_eval_multi_key/migration.sql"),
    sha256: "b979459e5949f3731ac4ec029665748e312f8420a6756c9775b99fcd9210cf7c",
};

pub(super) const ADD_DAILY_GUARDRAIL_UNITS: MigrationFile = MigrationFile {
    directory: "20260817143646_add_daily_guardrail_usage_units",
    bytes: include_bytes!(
        "../../migrations/20260817143646_add_daily_guardrail_usage_units/migration.sql"
    ),
    sha256: "c49f1741928eb7c91f6f1c65a8795dc34ed337f4bf287dacb394dbdb21da3d2e",
};

pub(super) const ADD_SPEND_LOG_TIMESTAMPS: MigrationFile = MigrationFile {
    directory: "20260818000000_add_spend_log_timestamps",
    bytes: include_bytes!("../../migrations/20260818000000_add_spend_log_timestamps/migration.sql"),
    sha256: "43483d9059a5bb4c5d1909b66cf31a0bcfcb56ee8cee41759a2fe6cc7ec29f7a",
};

pub(super) const ADD_SHADOW_EVAL_STOPPED_BY: MigrationFile = MigrationFile {
    directory: "20260818224500_add_shadow_eval_stopped_by",
    bytes: include_bytes!(
        "../../migrations/20260818224500_add_shadow_eval_stopped_by/migration.sql"
    ),
    sha256: "eb7eab0f28f0195e29aa3b522dadd5f20cab937321b28f9414694c77ba7275d2",
};

pub(super) const SHADOW_EVAL_MAX_BUDGET: MigrationFile = MigrationFile {
    directory: "20260819000000_shadow_eval_max_budget",
    bytes: include_bytes!("../../migrations/20260819000000_shadow_eval_max_budget/migration.sql"),
    sha256: "57b13e4db3145e309bc1de0f6227ca1ac996b770ed64a305e8d5d576339c6dce",
};

pub(super) const ADD_SPEND_LOG_KEY_START_IDX: MigrationFile = MigrationFile {
    directory: "20260823000000_add_spend_logs_api_key_starttime_index",
    bytes: include_bytes!(
        "../../migrations/20260823000000_add_spend_logs_api_key_starttime_index/migration.sql"
    ),
    sha256: "2ae38bee90bda6ecc16d6cfc4cd6c33344b47557a4ed1b35eaabf58b02540ce0",
};

pub(super) const ADD_CACHE_SAVINGS_SPEND: MigrationFile = MigrationFile {
    directory: "20260824000000_add_gateway_injected_caching_savings_spend",
    bytes: include_bytes!(
        "../../migrations/20260824000000_add_gateway_injected_caching_savings_spend/migration.sql"
    ),
    sha256: "9ae4b5a13c7e655716429ce5e54e48e4526ba0da3eac15b2638922636ea43604",
};

pub(super) const SHADOW_EVAL_COST_COMPARISON: MigrationFile = MigrationFile {
    directory: "20260828000000_shadow_eval_cost_comparison",
    bytes: include_bytes!(
        "../../migrations/20260828000000_shadow_eval_cost_comparison/migration.sql"
    ),
    sha256: "57e6e28b853b51674dc0161a71cb1149f449086cd7ee787e5eb768283fed25a9",
};

pub(super) const ADD_ACCESS_GROUP_BUDGET: MigrationFile = MigrationFile {
    directory: "20260829000000_add_model_access_group_budget_table",
    bytes: include_bytes!(
        "../../migrations/20260829000000_add_model_access_group_budget_table/migration.sql"
    ),
    sha256: "d8003a7d0a4fdf580aaf1262408959882b205ee83a13bd131ca706fd3b3e1298",
};

pub(super) const SHADOW_EVAL_TYPED_TARGETS: MigrationFile = MigrationFile {
    directory: "20260831000000_shadow_eval_typed_targets",
    bytes: include_bytes!(
        "../../migrations/20260831000000_shadow_eval_typed_targets/migration.sql"
    ),
    sha256: "78357ae2db2ee3b5305d5630c0eae95af59ae2db07e5adf21b5fb4f37f334cd6",
};

pub(super) const ADD_LITELLM_CALL_ID_SPEND_LOGS: MigrationFile = MigrationFile {
    directory: "20260831120000_add_litellm_call_id_spend_logs",
    bytes: include_bytes!(
        "../../migrations/20260831120000_add_litellm_call_id_spend_logs/migration.sql"
    ),
    sha256: "bcb344e885c0784de365a94646a21a5564942a90b55bcbcc9e3c492fb2b831b6",
};

pub(super) const SPEND_LOG_CALL_ID_INDEX: MigrationFile = MigrationFile {
    directory: "20260831120001_spend_logs_litellm_call_id_index",
    bytes: include_bytes!(
        "../../migrations/20260831120001_spend_logs_litellm_call_id_index/migration.sql"
    ),
    sha256: "7166fd213126f8dcbd6d1ecdf0d2851cdd55abf5e518242f2f44bb6b100a872d",
};

pub(super) const SHADOW_EVAL_MULTI_ROUTER: MigrationFile = MigrationFile {
    directory: "20260901000000_shadow_eval_multi_router",
    bytes: include_bytes!("../../migrations/20260901000000_shadow_eval_multi_router/migration.sql"),
    sha256: "f6980539317506b5daa6746011743bf8f369a9f0d6a4469d696be46faae453f2",
};

pub(super) const ADD_GUARDRAIL_USAGE_UNITS_COST: MigrationFile = MigrationFile {
    directory: "20260901000001_add_guardrail_usage_units_cost",
    bytes: include_bytes!(
        "../../migrations/20260901000001_add_guardrail_usage_units_cost/migration.sql"
    ),
    sha256: "7381f8ea473e50d0dcb67aff508a6fe70984da9ec3d821652db87afe39733106",
};

pub(super) const ADD_ORG_ID_MANAGED_OBJECT: MigrationFile = MigrationFile {
    directory: "20260903230000_add_org_id_to_managed_object_table",
    bytes: include_bytes!(
        "../../migrations/20260903230000_add_org_id_to_managed_object_table/migration.sql"
    ),
    sha256: "35fd128554221aa4409cc8405b990c70c37b7a291dd00f7ab766b0d5a52cc977",
};

pub(super) const SHADOW_EVAL_MODEL_SCOPE: MigrationFile = MigrationFile {
    directory: "20260904000000_shadow_eval_model_scope",
    bytes: include_bytes!("../../migrations/20260904000000_shadow_eval_model_scope/migration.sql"),
    sha256: "f58035b900234c4a2d93691cdeef0b033409eacfb8bebecac978c3b7b21084e5",
};

pub(super) const ADD_MCP_OAUTH_DISCOVERY: MigrationFile = MigrationFile {
    directory: "20260905120000_add_per_server_oauth_discovery_to_mcp_servers",
    bytes: include_bytes!(
        "../../migrations/20260905120000_add_per_server_oauth_discovery_to_mcp_servers/migration.sql"
    ),
    sha256: "f18e82c054d69d040e4eeb6b1a036638216117b5d2292cb7cc315f8ffc217da6",
};

pub(super) const ADD_AUTOROUTER_CLASSIFIER_COST: MigrationFile = MigrationFile {
    directory: "20260907000000_add_autorouter_classifier_cost",
    bytes: include_bytes!(
        "../../migrations/20260907000000_add_autorouter_classifier_cost/migration.sql"
    ),
    sha256: "af862ad1c02eb4467747610cec938d50549df28a9ad0810f0a8ddde3644f7c8f",
};

pub(super) const ADD_AUTOROUTER_BASE_MODELS: MigrationFile = MigrationFile {
    directory: "20260910000000_add_autorouter_session_baseline_models",
    bytes: include_bytes!(
        "../../migrations/20260910000000_add_autorouter_session_baseline_models/migration.sql"
    ),
    sha256: "cb6444896cbeb60aa289c10750c65eb5a7a2fc9baa2552394e047658cf03decf",
};

pub(super) const SKILLS_ON_OBJECT_PERMISSION: MigrationFile = MigrationFile {
    directory: "20260910000000_skills_on_object_permission",
    bytes: include_bytes!(
        "../../migrations/20260910000000_skills_on_object_permission/migration.sql"
    ),
    sha256: "8bb13d391be3872bc38b9455224292bd55f7179f4f4f9f2065659072d1e0e208",
};

pub(super) const ADD_TPD_LIMIT: MigrationFile = MigrationFile {
    directory: "20260913000000_add_tpd_limit",
    bytes: include_bytes!("../../migrations/20260913000000_add_tpd_limit/migration.sql"),
    sha256: "e65f7d0fa6d24cf46fc6596aed9371d7f49ff9d45670ac36b542d8395a84d57b",
};

pub(super) const ADD_AUTOROUTER_COVERAGE: MigrationFile = MigrationFile {
    directory: "20260915000000_add_autorouter_savings_estimate_coverage",
    bytes: include_bytes!(
        "../../migrations/20260915000000_add_autorouter_savings_estimate_coverage/migration.sql"
    ),
    sha256: "426f1b80ffe1319401bd994a9bdc061145a09fd080e9bf074bf8f22980a238ab",
};

pub(super) const ADD_DAILY_GLOBAL_SPEND: MigrationFile = MigrationFile {
    directory: "20260915000000_add_daily_global_spend",
    bytes: include_bytes!("../../migrations/20260915000000_add_daily_global_spend/migration.sql"),
    sha256: "83d716d3adf6eeee67eed7b037aa975b92317d38691dc6e0230616fcaeec3bda",
};

pub(super) const ADD_DAILY_RESPONSE_TIME: MigrationFile = MigrationFile {
    directory: "20260915000000_add_daily_response_time",
    bytes: include_bytes!("../../migrations/20260915000000_add_daily_response_time/migration.sql"),
    sha256: "b19733a85da03b52b6265d3196781e4e4131beb6ffc76942136cb361a1da67f3",
};

pub(super) const SCOPE_JWT_KEY_BY_ISSUER: MigrationFile = MigrationFile {
    directory: "20260915000000_scope_jwt_key_mapping_by_issuer",
    bytes: include_bytes!(
        "../../migrations/20260915000000_scope_jwt_key_mapping_by_issuer/migration.sql"
    ),
    sha256: "9a2954a7a631243ea15d7437b86bc199d424cccc9f596c3223b5a408259636c3",
};

pub(super) const ADD_AUTOROUTER_BASELINE_STATE: MigrationFile = MigrationFile {
    directory: "20260915010000_add_autorouter_baseline_state",
    bytes: include_bytes!(
        "../../migrations/20260915010000_add_autorouter_baseline_state/migration.sql"
    ),
    sha256: "181dc0a452fb062476fe77e18351f48e20c2b14f763d3b19dce3ccb81c962d3d",
};

pub(super) const ADD_KEY_TOTAL_SPEND: MigrationFile = MigrationFile {
    directory: "20260916000000_add_key_total_spend",
    bytes: include_bytes!("../../migrations/20260916000000_add_key_total_spend/migration.sql"),
    sha256: "a6c0cd6606339cc5267c21d202b6f6142bdb47806d17075f09fcbd15ad967b5d",
};

pub(super) const ADD_AGENT_ACCESS_GROUP_IDS: MigrationFile = MigrationFile {
    directory: "20260917000000_add_agent_access_group_ids",
    bytes: include_bytes!(
        "../../migrations/20260917000000_add_agent_access_group_ids/migration.sql"
    ),
    sha256: "d40a9042f649e10aea65d170104f2cf12dbcfa3a9c6324b3f5f73e578ca32429",
};

pub(super) const ADD_BUDGET_TEMP_INCREASE: MigrationFile = MigrationFile {
    directory: "20260917000000_add_budget_temp_increase",
    bytes: include_bytes!("../../migrations/20260917000000_add_budget_temp_increase/migration.sql"),
    sha256: "5fd45400bee66355df8cc4dc7b839cdcbff2555de2e0a87f508051b3ef49463e",
};

pub(super) const ADD_POLICY_ATTACHMENT_PRIORITY: MigrationFile = MigrationFile {
    directory: "20260917055603_add_policy_attachment_priority",
    bytes: include_bytes!(
        "../../migrations/20260917055603_add_policy_attachment_priority/migration.sql"
    ),
    sha256: "9591135a8f8899d8129b100b287776fe0c52cbb0b89f0e05057716359b3d953f",
};

pub(super) const ADD_MANAGED_FILE_CONTENT_TABLE: MigrationFile = MigrationFile {
    directory: "20260918000000_add_managed_file_content_table",
    bytes: include_bytes!(
        "../../migrations/20260918000000_add_managed_file_content_table/migration.sql"
    ),
    sha256: "d8c32459c92896d2e7ba57c7541da5449aa2c7124b02bb787bf3b221c3601849",
};

pub(super) const ADD_AUTOROUTER_USER_ROLLUP: MigrationFile = MigrationFile {
    directory: "20260919000000_add_autorouter_user_session_rollup",
    bytes: include_bytes!(
        "../../migrations/20260919000000_add_autorouter_user_session_rollup/migration.sql"
    ),
    sha256: "d5227cb7dbe124069aef53dd1050ad50853eec3b134eec2fd16917b302d8fa22",
};

pub(super) const ADD_POLICY_ATTACHMENT_DEFAULT: MigrationFile = MigrationFile {
    directory: "20260920041500_add_policy_attachment_is_default",
    bytes: include_bytes!(
        "../../migrations/20260920041500_add_policy_attachment_is_default/migration.sql"
    ),
    sha256: "33259c7ebc3c2a2d7f2d81eef1e8f3fb94217facd882694bec2c30f9b7d851ef",
};

pub(super) const ADD_PASSWORD_RESET_COLUMNS: MigrationFile = MigrationFile {
    directory: "20260921000000_add_password_reset_columns",
    bytes: include_bytes!(
        "../../migrations/20260921000000_add_password_reset_columns/migration.sql"
    ),
    sha256: "601287a72ed29f3813d97dd51a16a83407a585731788ab512fedf8a01f56a756",
};
