use super::{BEGIN_SQL, COMMIT_SQL, SpendPartitionStep};

pub(super) const STEPS: &[SpendPartitionStep] = &[
    SpendPartitionStep {
        label: "beginning the transaction",
        sql: BEGIN_SQL,
    },
    SpendPartitionStep {
        label: "renaming LiteLLM_SpendLogs to LiteLLM_SpendLogs_legacy",
        sql: r#"ALTER TABLE "LiteLLM_SpendLogs" RENAME TO "LiteLLM_SpendLogs_legacy""#,
    },
    SpendPartitionStep {
        label: "renaming LiteLLM_SpendLogs_pkey to LiteLLM_SpendLogs_legacy_pkey",
        sql: r#"ALTER INDEX IF EXISTS "LiteLLM_SpendLogs_pkey" RENAME TO "LiteLLM_SpendLogs_legacy_pkey""#,
    },
    SpendPartitionStep {
        label: "renaming LiteLLM_SpendLogs_startTime_idx to the legacy name",
        sql: r#"ALTER INDEX IF EXISTS "LiteLLM_SpendLogs_startTime_idx" RENAME TO "LiteLLM_SpendLogs_legacy_startTime_idx""#,
    },
    SpendPartitionStep {
        label: "renaming LiteLLM_SpendLogs_startTime_request_id_idx to the legacy name",
        sql: r#"ALTER INDEX IF EXISTS "LiteLLM_SpendLogs_startTime_request_id_idx" RENAME TO "LiteLLM_SpendLogs_legacy_startTime_request_id_idx""#,
    },
    SpendPartitionStep {
        label: "renaming LiteLLM_SpendLogs_end_user_idx to the legacy name",
        sql: r#"ALTER INDEX IF EXISTS "LiteLLM_SpendLogs_end_user_idx" RENAME TO "LiteLLM_SpendLogs_legacy_end_user_idx""#,
    },
    SpendPartitionStep {
        label: "renaming LiteLLM_SpendLogs_session_id_idx to the legacy name",
        sql: r#"ALTER INDEX IF EXISTS "LiteLLM_SpendLogs_session_id_idx" RENAME TO "LiteLLM_SpendLogs_legacy_session_id_idx""#,
    },
    SpendPartitionStep {
        label: "renaming LiteLLM_SpendLogs_api_key_startTime_idx to the legacy name",
        sql: r#"ALTER INDEX IF EXISTS "LiteLLM_SpendLogs_api_key_startTime_idx" RENAME TO "LiteLLM_SpendLogs_legacy_api_key_startTime_idx""#,
    },
    SpendPartitionStep {
        label: "creating the partitioned LiteLLM_SpendLogs",
        sql: r#"CREATE TABLE "LiteLLM_SpendLogs" (LIKE "LiteLLM_SpendLogs_legacy" INCLUDING DEFAULTS INCLUDING GENERATED) PARTITION BY RANGE ("startTime")"#,
    },
    SpendPartitionStep {
        label: "adding the composite primary key",
        sql: r#"ALTER TABLE "LiteLLM_SpendLogs" ADD PRIMARY KEY ("request_id", "startTime")"#,
    },
    SpendPartitionStep {
        label: "creating LiteLLM_SpendLogs_startTime_idx",
        sql: r#"CREATE INDEX IF NOT EXISTS "LiteLLM_SpendLogs_startTime_idx" ON "LiteLLM_SpendLogs" ("startTime")"#,
    },
    SpendPartitionStep {
        label: "creating LiteLLM_SpendLogs_startTime_request_id_idx",
        sql: r#"CREATE INDEX IF NOT EXISTS "LiteLLM_SpendLogs_startTime_request_id_idx" ON "LiteLLM_SpendLogs" ("startTime", "request_id")"#,
    },
    SpendPartitionStep {
        label: "creating LiteLLM_SpendLogs_end_user_idx",
        sql: r#"CREATE INDEX IF NOT EXISTS "LiteLLM_SpendLogs_end_user_idx" ON "LiteLLM_SpendLogs" ("end_user")"#,
    },
    SpendPartitionStep {
        label: "creating LiteLLM_SpendLogs_session_id_idx",
        sql: r#"CREATE INDEX IF NOT EXISTS "LiteLLM_SpendLogs_session_id_idx" ON "LiteLLM_SpendLogs" ("session_id")"#,
    },
    SpendPartitionStep {
        label: "creating LiteLLM_SpendLogs_api_key_startTime_idx",
        sql: r#"CREATE INDEX IF NOT EXISTS "LiteLLM_SpendLogs_api_key_startTime_idx" ON "LiteLLM_SpendLogs" ("api_key", "startTime")"#,
    },
    SpendPartitionStep {
        label: "creating the default partition",
        sql: r#"CREATE TABLE IF NOT EXISTS "LiteLLM_SpendLogs_pdefault" PARTITION OF "LiteLLM_SpendLogs" DEFAULT"#,
    },
    SpendPartitionStep {
        label: "committing the transaction",
        sql: COMMIT_SQL,
    },
];
