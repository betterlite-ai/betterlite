// the judged attempts of the job's legs, grouped by the tier that classified each real request
pub(crate) const ATTEMPT_AGG_BY_TIER_SQL: &str = r#"SELECT COALESCE("tier", 'UNCLASSIFIED') AS "grp",
    COUNT(*)::int AS "turn_count",
    COUNT(*) FILTER (WHERE "outcome" = 'real')::int AS "real_wins",
    COUNT(*) FILTER (WHERE "outcome" = 'shadow')::int AS "shadow_wins",
    COUNT(*) FILTER (WHERE "outcome" = 'tie')::int AS "ties",
    AVG("confidence")::float AS "avg_confidence",
    COALESCE(SUM("real_cost" + "real_classifier_cost") FILTER (WHERE "real_cost" IS NOT NULL AND NOT "real_cache_hit"), 0)::float AS "real_spend",
    COALESCE(SUM("shadow_cost" + "shadow_classifier_cost") FILTER (WHERE "real_cost" IS NOT NULL AND NOT "real_cache_hit"), 0)::float AS "shadow_spend",
    COUNT(*) FILTER (WHERE "real_cache_hit")::int AS "cache_hit_turns"
FROM "LiteLLM_ShadowEvalAttempt"
WHERE "job_id" = ANY($1::text[]) AND "outcome" != 'error'
GROUP BY 1"#;

pub(crate) const ATTEMPT_AGG_BY_MODEL_SQL: &str = r#"SELECT COALESCE("real_model", 'unknown') AS "grp",
    COUNT(*)::int AS "turn_count",
    COUNT(*) FILTER (WHERE "outcome" = 'real')::int AS "real_wins",
    COUNT(*) FILTER (WHERE "outcome" = 'shadow')::int AS "shadow_wins",
    COUNT(*) FILTER (WHERE "outcome" = 'tie')::int AS "ties",
    AVG("confidence")::float AS "avg_confidence",
    COALESCE(SUM("real_cost" + "real_classifier_cost") FILTER (WHERE "real_cost" IS NOT NULL AND NOT "real_cache_hit"), 0)::float AS "real_spend",
    COALESCE(SUM("shadow_cost" + "shadow_classifier_cost") FILTER (WHERE "real_cost" IS NOT NULL AND NOT "real_cache_hit"), 0)::float AS "shadow_spend",
    COUNT(*) FILTER (WHERE "real_cache_hit")::int AS "cache_hit_turns"
FROM "LiteLLM_ShadowEvalAttempt"
WHERE "job_id" = ANY($1::text[]) AND "outcome" != 'error'
GROUP BY 1"#;

pub(crate) const ATTEMPT_AGG_BY_LEG_SQL: &str = r#"SELECT "job_id" AS "grp",
    COUNT(*)::int AS "turn_count",
    COUNT(*) FILTER (WHERE "outcome" = 'real')::int AS "real_wins",
    COUNT(*) FILTER (WHERE "outcome" = 'shadow')::int AS "shadow_wins",
    COUNT(*) FILTER (WHERE "outcome" = 'tie')::int AS "ties",
    AVG("confidence")::float AS "avg_confidence",
    COALESCE(SUM("real_cost" + "real_classifier_cost") FILTER (WHERE "real_cost" IS NOT NULL AND NOT "real_cache_hit"), 0)::float AS "real_spend",
    COALESCE(SUM("shadow_cost" + "shadow_classifier_cost") FILTER (WHERE "real_cost" IS NOT NULL AND NOT "real_cache_hit"), 0)::float AS "shadow_spend",
    COUNT(*) FILTER (WHERE "real_cache_hit")::int AS "cache_hit_turns"
FROM "LiteLLM_ShadowEvalAttempt"
WHERE "job_id" = ANY($1::text[]) AND "outcome" != 'error'
GROUP BY 1"#;

// rows from before arm stamping carry no router_name; they belong to the job's own router
pub(crate) const ATTEMPT_AGG_BY_ROUTER_SQL: &str = r#"SELECT COALESCE(a."router_name", j."router_name") AS "grp",
    COUNT(*)::int AS "turn_count",
    COUNT(*) FILTER (WHERE a."outcome" = 'real')::int AS "real_wins",
    COUNT(*) FILTER (WHERE a."outcome" = 'shadow')::int AS "shadow_wins",
    COUNT(*) FILTER (WHERE a."outcome" = 'tie')::int AS "ties",
    AVG(a."confidence")::float AS "avg_confidence",
    COALESCE(SUM(a."real_cost" + a."real_classifier_cost") FILTER (WHERE a."real_cost" IS NOT NULL AND NOT a."real_cache_hit"), 0)::float AS "real_spend",
    COALESCE(SUM(a."shadow_cost" + a."shadow_classifier_cost") FILTER (WHERE a."real_cost" IS NOT NULL AND NOT a."real_cache_hit"), 0)::float AS "shadow_spend",
    COUNT(*) FILTER (WHERE a."real_cache_hit")::int AS "cache_hit_turns"
FROM "LiteLLM_ShadowEvalAttempt" a
JOIN "LiteLLM_ShadowEvalJob" j ON j."id" = a."job_id"
WHERE a."job_id" = ANY($1::text[]) AND a."outcome" != 'error'
GROUP BY 1"#;

// every attempt counts toward judge spend, so an errored turn still reports what it cost
pub(crate) const ATTEMPT_TOTALS_SQL: &str = r#"SELECT
    COUNT(*) FILTER (WHERE "outcome" != 'error')::int AS "judged_count",
    COUNT(*) FILTER (WHERE "outcome" = 'error')::int AS "error_count",
    COALESCE(SUM("judge_cost"), 0)::float AS "judge_spend"
FROM "LiteLLM_ShadowEvalAttempt"
WHERE "job_id" = ANY($1::text[])"#;

// a stamped leg's figures freeze at stopped_at, so a late attempt can never reclassify it
pub(crate) const ATTEMPT_COUNTS_SQL: &str = r#"SELECT a."job_id", COUNT(*)::int AS "attempt_count",
       COALESCE(SUM(a."judge_cost" + a."shadow_cost" + a."shadow_classifier_cost"), 0)::float AS "spend"
FROM "LiteLLM_ShadowEvalAttempt" a
JOIN "LiteLLM_ShadowEvalJob" j ON j."id" = a."job_id"
WHERE a."job_id" = ANY($1::text[]) AND (j."stopped_at" IS NULL OR a."created_at" <= j."stopped_at")
GROUP BY a."job_id""#;

pub(crate) const FUNNEL_TOTALS_SQL: &str = r#"SELECT COUNT(*)::int AS "legs_with_rows",
    COALESCE(SUM("not_sampled"), 0)::int AS "not_sampled",
    COALESCE(SUM("unjudgeable"), 0)::int AS "unjudgeable",
    COALESCE(SUM("shed"), 0)::int AS "shed",
    COALESCE(SUM("withheld"), 0)::int AS "withheld"
FROM "LiteLLM_ShadowEvalFunnel"
WHERE "job_id" = ANY($1::text[])"#;

// the newest errored attempt of each leg; the caller picks the newest of those across the job
pub(crate) const LATEST_ERROR_ROWS_SQL: &str = r#"SELECT DISTINCT ON ("job_id") "error", "created_at"
FROM "LiteLLM_ShadowEvalAttempt"
WHERE "job_id" = ANY($1::text[]) AND "outcome" = 'error'
ORDER BY "job_id", "created_at" DESC"#;

pub(crate) const KEY_LABELS_SQL: &str = r#"SELECT "token" AS "target_id", "key_alias", "key_name"
FROM "LiteLLM_VerificationToken"
WHERE "token" = ANY($1::text[])"#;

pub(crate) const TEAM_LABELS_SQL: &str = r#"SELECT "team_id" AS "target_id", "team_alias"
FROM "LiteLLM_TeamTable"
WHERE "team_id" = ANY($1::text[])"#;

pub(crate) const USER_LABELS_SQL: &str = r#"SELECT "user_id" AS "target_id", "user_email"
FROM "LiteLLM_UserTable"
WHERE "user_id" = ANY($1::text[])"#;
