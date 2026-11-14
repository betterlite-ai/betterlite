use super::{BEGIN_SQL, COMMIT_SQL, SpendPartitionStep};

pub(super) const STEPS: &[SpendPartitionStep] = &[
    SpendPartitionStep {
        label: "beginning the transaction",
        sql: BEGIN_SQL,
    },
    SpendPartitionStep {
        label: "renaming LiteLLM_SpendLogs to LiteLLM_SpendLogs_partitioned",
        sql: r#"ALTER TABLE "LiteLLM_SpendLogs" RENAME TO "LiteLLM_SpendLogs_partitioned""#,
    },
    SpendPartitionStep {
        label: "renaming LiteLLM_SpendLogs_pkey to the partitioned name",
        sql: r#"ALTER INDEX IF EXISTS "LiteLLM_SpendLogs_pkey" RENAME TO "LiteLLM_SpendLogs_partitioned_pkey""#,
    },
    SpendPartitionStep {
        label: "renaming LiteLLM_SpendLogs_pkey1 to the partitioned name",
        sql: r#"ALTER INDEX IF EXISTS "LiteLLM_SpendLogs_pkey1" RENAME TO "LiteLLM_SpendLogs_partitioned_pkey1""#,
    },
    SpendPartitionStep {
        label: "renaming LiteLLM_SpendLogs_startTime_idx to the partitioned name",
        sql: r#"ALTER INDEX IF EXISTS "LiteLLM_SpendLogs_startTime_idx" RENAME TO "LiteLLM_SpendLogs_partitioned_startTime_idx""#,
    },
    SpendPartitionStep {
        label: "renaming LiteLLM_SpendLogs_startTime_request_id_idx to the partitioned name",
        sql: r#"ALTER INDEX IF EXISTS "LiteLLM_SpendLogs_startTime_request_id_idx" RENAME TO "LiteLLM_SpendLogs_partitioned_startTime_request_id_idx""#,
    },
    SpendPartitionStep {
        label: "renaming LiteLLM_SpendLogs_end_user_idx to the partitioned name",
        sql: r#"ALTER INDEX IF EXISTS "LiteLLM_SpendLogs_end_user_idx" RENAME TO "LiteLLM_SpendLogs_partitioned_end_user_idx""#,
    },
    SpendPartitionStep {
        label: "renaming LiteLLM_SpendLogs_session_id_idx to the partitioned name",
        sql: r#"ALTER INDEX IF EXISTS "LiteLLM_SpendLogs_session_id_idx" RENAME TO "LiteLLM_SpendLogs_partitioned_session_id_idx""#,
    },
    SpendPartitionStep {
        label: "renaming LiteLLM_SpendLogs_api_key_startTime_idx to the partitioned name",
        sql: r#"ALTER INDEX IF EXISTS "LiteLLM_SpendLogs_api_key_startTime_idx" RENAME TO "LiteLLM_SpendLogs_partitioned_api_key_startTime_idx""#,
    },
    SpendPartitionStep {
        label: "creating the plain LiteLLM_SpendLogs",
        sql: r#"CREATE TABLE "LiteLLM_SpendLogs" (LIKE "LiteLLM_SpendLogs_partitioned" INCLUDING DEFAULTS INCLUDING GENERATED)"#,
    },
    SpendPartitionStep {
        label: "restoring the single-column primary key",
        sql: r#"ALTER TABLE "LiteLLM_SpendLogs" ADD PRIMARY KEY ("request_id")"#,
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
        label: "copying rows into LiteLLM_SpendLogs",
        sql: r#"INSERT INTO "LiteLLM_SpendLogs" SELECT * FROM "LiteLLM_SpendLogs_partitioned" ON CONFLICT ("request_id") DO NOTHING"#,
    },
    SpendPartitionStep {
        label: "dropping LiteLLM_SpendLogs_partitioned",
        sql: r#"DROP TABLE "LiteLLM_SpendLogs_partitioned""#,
    },
    SpendPartitionStep {
        label: "committing the transaction",
        sql: COMMIT_SQL,
    },
];
