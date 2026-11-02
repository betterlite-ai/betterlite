use crate::schema_baseline::MigrationCatalog;

pub mod fixtures;
mod migrations;

pub const MIGRATION_CATALOG: MigrationCatalog = MigrationCatalog {
    files: &[
        migrations::ADD_SHADOW_EVAL,
        migrations::ADD_SHADOW_EVAL_DIRECTION,
        migrations::ADD_PROXY_WORKER_HEARTBEAT,
        migrations::SHADOW_EVAL_MULTI_KEY,
        migrations::ADD_DAILY_GUARDRAIL_UNITS,
        migrations::ADD_SPEND_LOG_TIMESTAMPS,
        migrations::ADD_SHADOW_EVAL_STOPPED_BY,
        migrations::SHADOW_EVAL_MAX_BUDGET,
        migrations::ADD_SPEND_LOG_KEY_START_IDX,
        migrations::ADD_CACHE_SAVINGS_SPEND,
        migrations::SHADOW_EVAL_COST_COMPARISON,
        migrations::ADD_ACCESS_GROUP_BUDGET,
        migrations::SHADOW_EVAL_TYPED_TARGETS,
        migrations::ADD_LITELLM_CALL_ID_SPEND_LOGS,
        migrations::SPEND_LOG_CALL_ID_INDEX,
        migrations::SHADOW_EVAL_MULTI_ROUTER,
        migrations::ADD_GUARDRAIL_USAGE_UNITS_COST,
        migrations::ADD_ORG_ID_MANAGED_OBJECT,
        migrations::SHADOW_EVAL_MODEL_SCOPE,
        migrations::ADD_MCP_OAUTH_DISCOVERY,
        migrations::ADD_AUTOROUTER_CLASSIFIER_COST,
        migrations::ADD_AUTOROUTER_BASE_MODELS,
        migrations::SKILLS_ON_OBJECT_PERMISSION,
        migrations::ADD_TPD_LIMIT,
        migrations::ADD_AUTOROUTER_COVERAGE,
        migrations::ADD_DAILY_GLOBAL_SPEND,
        migrations::ADD_DAILY_RESPONSE_TIME,
        migrations::SCOPE_JWT_KEY_BY_ISSUER,
        migrations::ADD_AUTOROUTER_BASELINE_STATE,
        migrations::ADD_KEY_TOTAL_SPEND,
        migrations::ADD_AGENT_ACCESS_GROUP_IDS,
        migrations::ADD_BUDGET_TEMP_INCREASE,
        migrations::ADD_POLICY_ATTACHMENT_PRIORITY,
        migrations::ADD_MANAGED_FILE_CONTENT_TABLE,
        migrations::ADD_AUTOROUTER_USER_ROLLUP,
        migrations::ADD_POLICY_ATTACHMENT_DEFAULT,
        migrations::ADD_PASSWORD_RESET_COLUMNS,
    ],
};

mod tests;
