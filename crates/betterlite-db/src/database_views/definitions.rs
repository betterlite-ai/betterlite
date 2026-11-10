#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewDefinition {
    pub name: &'static str,
    pub ddl: &'static str,
}
pub const VERIFICATION_TOKEN_VIEW: &str = "LiteLLM_VerificationTokenView";
pub const MONTHLY_GLOBAL_SPEND: &str = "MonthlyGlobalSpend";
pub const LAST_30D_KEYS_BY_SPEND: &str = "Last30dKeysBySpend";
pub const LAST_30D_MODELS_BY_SPEND: &str = "Last30dModelsBySpend";
pub const MONTHLY_GLOBAL_SPEND_PER_KEY: &str = "MonthlyGlobalSpendPerKey";
pub const MONTHLY_USER_KEY_SPEND: &str = "MonthlyGlobalSpendPerUserPerKey";
pub const DAILY_TAG_SPEND: &str = "DailyTagSpend";
pub const LAST_30D_TOP_END_USERS_SPEND: &str = "Last30dTopEndUsersSpend";
pub const RUNTIME_VERIFICATION_VIEW_SQL: &str = r#"CREATE VIEW "LiteLLM_VerificationTokenView" AS
SELECT v.*, t.spend AS team_spend, t.max_budget AS team_max_budget, t.model_max_budget AS team_model_max_budget, t.tpm_limit AS team_tpm_limit, t.rpm_limit AS team_rpm_limit, t.tpd_limit AS team_tpd_limit, p.project_alias AS project_alias
FROM "LiteLLM_VerificationToken" v
LEFT JOIN "LiteLLM_TeamTable" t ON v.team_id = t.team_id
LEFT JOIN "LiteLLM_ProjectTable" p ON v.project_id = p.project_id;"#;
pub const OFFLINE_VERIFICATION_VIEW_SQL: &str = r#"CREATE VIEW "LiteLLM_VerificationTokenView" AS
SELECT v.*, t.spend AS team_spend, t.max_budget AS team_max_budget, t.tpm_limit AS team_tpm_limit, t.rpm_limit AS team_rpm_limit
FROM "LiteLLM_VerificationToken" v
LEFT JOIN "LiteLLM_TeamTable" t ON v.team_id = t.team_id;"#;
pub const MONTHLY_GLOBAL_SPEND_SQL: &str = r#"CREATE OR REPLACE VIEW "MonthlyGlobalSpend" AS
SELECT DATE("startTime") AS date, SUM("spend") AS spend
FROM "LiteLLM_SpendLogs"
WHERE "startTime" >= (CURRENT_DATE - INTERVAL '30 days')
GROUP BY DATE("startTime");"#;
pub const LAST_30D_KEYS_BY_SPEND_SQL: &str = r#"CREATE OR REPLACE VIEW "Last30dKeysBySpend" AS
SELECT L."api_key", V."key_alias", V."key_name", SUM(L."spend") AS total_spend
FROM "LiteLLM_SpendLogs" L
LEFT JOIN "LiteLLM_VerificationToken" V ON L."api_key" = V."token"
WHERE L."startTime" >= (CURRENT_DATE - INTERVAL '30 days')
GROUP BY L."api_key", V."key_alias", V."key_name"
ORDER BY total_spend DESC;"#;
pub const LAST_30D_MODELS_BY_SPEND_SQL: &str = r#"CREATE OR REPLACE VIEW "Last30dModelsBySpend" AS
SELECT "model", SUM("spend") AS total_spend
FROM "LiteLLM_SpendLogs"
WHERE "startTime" >= (CURRENT_DATE - INTERVAL '30 days') AND "model" != ''
GROUP BY "model"
ORDER BY total_spend DESC;"#;
pub const SPEND_PER_KEY_SQL: &str = r#"CREATE OR REPLACE VIEW "MonthlyGlobalSpendPerKey" AS
SELECT DATE("startTime") AS date, SUM("spend") AS spend, api_key as api_key
FROM "LiteLLM_SpendLogs"
WHERE "startTime" >= (CURRENT_DATE - INTERVAL '30 days')
GROUP BY DATE("startTime"), api_key;"#;
pub const USER_KEY_SPEND_SQL: &str = r#"CREATE OR REPLACE VIEW "MonthlyGlobalSpendPerUserPerKey" AS
SELECT DATE("startTime") AS date, SUM("spend") AS spend, api_key as api_key, "user" as "user"
FROM "LiteLLM_SpendLogs"
WHERE "startTime" >= (CURRENT_DATE - INTERVAL '30 days')
GROUP BY DATE("startTime"), "user", api_key;"#;
pub const DAILY_TAG_SPEND_SQL: &str = r#"CREATE OR REPLACE VIEW "DailyTagSpend" AS
SELECT jsonb_array_elements_text(request_tags) AS individual_request_tag, DATE(s."startTime") AS spend_date, COUNT(*) AS log_count, SUM(spend) AS total_spend
FROM "LiteLLM_SpendLogs" s
GROUP BY individual_request_tag, DATE(s."startTime");"#;
pub const TOP_END_USERS_SQL: &str = r#"CREATE VIEW "Last30dTopEndUsersSpend" AS
SELECT end_user, COUNT(*) AS total_events, SUM(spend) AS total_spend
FROM "LiteLLM_SpendLogs"
WHERE end_user <> '' AND end_user <> user AND "startTime" >= CURRENT_DATE - INTERVAL '30 days'
GROUP BY end_user
ORDER BY total_spend DESC
LIMIT 100;"#;
pub const RUNTIME_VIEWS: [ViewDefinition; 8] = [
    ViewDefinition {
        name: VERIFICATION_TOKEN_VIEW,
        ddl: RUNTIME_VERIFICATION_VIEW_SQL,
    },
    ViewDefinition {
        name: MONTHLY_GLOBAL_SPEND,
        ddl: MONTHLY_GLOBAL_SPEND_SQL,
    },
    ViewDefinition {
        name: LAST_30D_KEYS_BY_SPEND,
        ddl: LAST_30D_KEYS_BY_SPEND_SQL,
    },
    ViewDefinition {
        name: LAST_30D_MODELS_BY_SPEND,
        ddl: LAST_30D_MODELS_BY_SPEND_SQL,
    },
    ViewDefinition {
        name: MONTHLY_GLOBAL_SPEND_PER_KEY,
        ddl: SPEND_PER_KEY_SQL,
    },
    ViewDefinition {
        name: MONTHLY_USER_KEY_SPEND,
        ddl: USER_KEY_SPEND_SQL,
    },
    ViewDefinition {
        name: DAILY_TAG_SPEND,
        ddl: DAILY_TAG_SPEND_SQL,
    },
    ViewDefinition {
        name: LAST_30D_TOP_END_USERS_SPEND,
        ddl: TOP_END_USERS_SQL,
    },
];
pub const OFFLINE_VIEWS: [ViewDefinition; 8] = [
    ViewDefinition {
        name: VERIFICATION_TOKEN_VIEW,
        ddl: OFFLINE_VERIFICATION_VIEW_SQL,
    },
    ViewDefinition {
        name: MONTHLY_GLOBAL_SPEND,
        ddl: MONTHLY_GLOBAL_SPEND_SQL,
    },
    ViewDefinition {
        name: LAST_30D_KEYS_BY_SPEND,
        ddl: LAST_30D_KEYS_BY_SPEND_SQL,
    },
    ViewDefinition {
        name: LAST_30D_MODELS_BY_SPEND,
        ddl: LAST_30D_MODELS_BY_SPEND_SQL,
    },
    ViewDefinition {
        name: MONTHLY_GLOBAL_SPEND_PER_KEY,
        ddl: SPEND_PER_KEY_SQL,
    },
    ViewDefinition {
        name: MONTHLY_USER_KEY_SPEND,
        ddl: USER_KEY_SPEND_SQL,
    },
    ViewDefinition {
        name: DAILY_TAG_SPEND,
        ddl: DAILY_TAG_SPEND_SQL,
    },
    ViewDefinition {
        name: LAST_30D_TOP_END_USERS_SPEND,
        ddl: TOP_END_USERS_SQL,
    },
];
