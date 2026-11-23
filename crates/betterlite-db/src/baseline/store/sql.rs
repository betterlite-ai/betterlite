pub const CREATE_COMPARISON: &str = r#"
INSERT INTO "LiteLLM_AutoRouterBaselineComparison"
    (scope, api_key, session_id, router_name, initial_equivalent)
VALUES ($1, $2, $3, $4, NOT EXISTS (
    SELECT 1 FROM "LiteLLM_AutoRouterSession"
    WHERE api_key = $2 AND session_id = $3 AND router_name = $4
)) ON CONFLICT (scope) DO NOTHING
"#;

pub const LOCK_COMPARISON: &str = r#"
SELECT revision, published_revision, initial_equivalent, retired, history
FROM "LiteLLM_AutoRouterBaselineComparison" WHERE scope = $1 FOR UPDATE
"#;

pub const INSERT_RECORD: &str = r#"
INSERT INTO "LiteLLM_AutoRouterBaselineObservation"
    (request_id, scope, started_at, revision, data)
VALUES ($1, $2, $3::float8, $4::bigint, $5)
ON CONFLICT (request_id) DO NOTHING
"#;

pub const MARK_CONFLICT: &str = r#"
UPDATE "LiteLLM_AutoRouterBaselineObservation"
SET conflicted = TRUE, revision = $4::bigint
WHERE request_id = $1 AND scope = $2 AND data <> $3 AND NOT conflicted
"#;

pub const READ_CANONICAL: &str = r#"SELECT data, publication, conflicted, started_at FROM "LiteLLM_AutoRouterBaselineObservation" WHERE request_id=$1 AND scope=$2"#;

pub const ADVANCE_REVISION: &str = r#"UPDATE "LiteLLM_AutoRouterBaselineComparison" SET revision = $2::bigint, updated_at = CURRENT_TIMESTAMP, attempted_at = NULL WHERE scope = $1"#;

pub const READ_PAGE: &str = r#"
WITH times AS (
    SELECT DISTINCT started_at FROM "LiteLLM_AutoRouterBaselineObservation"
    WHERE scope = $1 AND revision > $2::bigint
      AND ($3::float8 IS NULL OR started_at > $3::float8)
      AND ($5::float8 IS NULL OR (
          started_at >= $5::float8 AND publication::jsonb->>'status' = 'estimated'
      ))
    ORDER BY started_at LIMIT $4::int
)
SELECT data, publication, conflicted, started_at
FROM "LiteLLM_AutoRouterBaselineObservation"
WHERE scope = $1 AND revision > $2::bigint
  AND started_at IN (SELECT started_at FROM times)
  AND ($5::float8 IS NULL OR publication::jsonb->>'status' = 'estimated')
ORDER BY started_at, request_id
"#;

pub const MISSING_LOG: &str = r#"SELECT 1 FROM "LiteLLM_AutoRouterBaselineObservation" AS observation WHERE scope=$1 AND publication IS NULL AND NOT EXISTS (SELECT 1 FROM "LiteLLM_SpendLogs" AS log WHERE log.request_id=observation.request_id) LIMIT 1"#;

pub const CHANGED_BEFORE: &str = r#"SELECT 1 FROM "LiteLLM_AutoRouterBaselineObservation" WHERE scope = $1 AND revision > $2::bigint AND started_at <= $3::float8 LIMIT 1"#;

pub const STORE_HISTORY: &str = r#"UPDATE "LiteLLM_AutoRouterBaselineComparison" SET published_revision = revision, history = $2 WHERE scope = $1"#;

pub const RETIRE_COMPARISONS: &str = r#"WITH expired AS (SELECT scope FROM "LiteLLM_AutoRouterBaselineComparison" WHERE NOT retired AND updated_at < $1::timestamptz ORDER BY updated_at LIMIT $2::int FOR UPDATE SKIP LOCKED) UPDATE "LiteLLM_AutoRouterBaselineComparison" AS comparison SET retired=TRUE, history=NULL FROM expired WHERE comparison.scope=expired.scope"#;

pub const DELETE_RETIRED: &str = r#"DELETE FROM "LiteLLM_AutoRouterBaselineObservation" WHERE request_id IN (SELECT event.request_id FROM "LiteLLM_AutoRouterBaselineObservation" AS event JOIN "LiteLLM_AutoRouterBaselineComparison" AS comparison USING (scope) WHERE comparison.retired AND comparison.updated_at < $1::timestamptz LIMIT $2::int)"#;

pub const CLAIM_DIRTY: &str = r#"
WITH candidates AS (
    SELECT scope FROM "LiteLLM_AutoRouterBaselineComparison"
    WHERE NOT retired AND revision <> published_revision
      AND (attempted_at IS NULL OR attempted_at < CURRENT_TIMESTAMP - INTERVAL '30 seconds')
    ORDER BY attempted_at NULLS FIRST, updated_at, scope LIMIT 32 FOR UPDATE SKIP LOCKED
)
UPDATE "LiteLLM_AutoRouterBaselineComparison" AS comparison
SET attempted_at = CURRENT_TIMESTAMP FROM candidates
WHERE comparison.scope = candidates.scope RETURNING comparison.scope
"#;
