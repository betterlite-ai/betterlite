use std::fmt::Write as _;

// the start endpoint's three target reads, each filtered by the ids the request named
pub const KEYS_SQL: &str = r#"SELECT "token", "key_alias", "key_name", "team_id"
FROM "LiteLLM_VerificationToken" WHERE "token" = ANY($1::text[])"#;

pub const TEAMS_SQL: &str = r#"SELECT "team_id", "team_alias"
FROM "LiteLLM_TeamTable" WHERE "team_id" = ANY($1::text[])"#;

pub const USERS_SQL: &str = r#"SELECT "user_id", "user_email"
FROM "LiteLLM_UserTable" WHERE "user_id" = ANY($1::text[])"#;

// _SWEEP_FINISHED_JOBS_SQL: stamp the legs a finished window, turn count or budget already ended, both directions
pub const SWEEP_FINISHED_SQL: &str = r#"UPDATE "LiteLLM_ShadowEvalJob" j SET stopped_at = (NOW() AT TIME ZONE 'utc')
WHERE j.target_type = $2 AND j.target_id = ANY($1::text[]) AND j.stopped_at IS NULL
  AND (
    j.ends_at <= (NOW() AT TIME ZONE 'utc')
    OR (SELECT COUNT(*) FROM "LiteLLM_ShadowEvalAttempt" a WHERE a.job_id = j.id) >= j.max_turns
    OR (
      j.max_budget IS NOT NULL
      AND (SELECT COALESCE(SUM(a.judge_cost + a.shadow_cost + a.shadow_classifier_cost), 0) FROM "LiteLLM_ShadowEvalAttempt" a WHERE a.job_id = j.id) >= j.max_budget
    )
  )"#;

// the claim read: any leg of a requested kind and id holding a slot in this direction, exact kinds never crossing
pub const CLAIMED_SQL: &str = r#"SELECT "group_id", "target_type", "target_id"
FROM "LiteLLM_ShadowEvalJob"
WHERE "stopped_at" IS NULL AND "direction" = $1
  AND (("target_type" = 'key' AND "target_id" = ANY($2::text[]))
    OR ("target_type" = 'team' AND "target_id" = ANY($3::text[]))
    OR ("target_type" = 'user' AND "target_id" = ANY($4::text[])))"#;

pub const INSERT_LEGS_HEAD: &str = r#"INSERT INTO "LiteLLM_ShadowEvalJob"
    ("id", "group_id", "target_type", "target_id", "router_name", "router_names", "models",
     "direction", "baseline_model", "judge_model", "shadow_percentage", "max_turns",
     "max_budget", "created_by", "created_at", "ends_at") VALUES "#;

pub const LEG_COLUMNS: usize = 16;

// zero-based slot of max_budget: Prisma sends the float, so Decimal travels through numeric
pub const MAX_BUDGET_SLOT: usize = 12;

// create_many(skip_duplicates=True): one statement, so a re-seed never fails a started job
pub const SEED_FUNNEL_SQL: &str = r#"INSERT INTO "LiteLLM_ShadowEvalFunnel" ("job_id")
SELECT unnest($1::text[]) ON CONFLICT ("job_id") DO NOTHING"#;

pub const LEGS_SQL: &str = r#"SELECT "id", "group_id", "target_type", "target_id", "router_name",
       "router_names", "models", "direction", "baseline_model", "judge_model", "shadow_percentage",
       "max_turns"::bigint AS "max_turns", "max_budget"::numeric AS "max_budget",
       "created_at", "ends_at", "stopped_at", "stopped_by"
FROM "LiteLLM_ShadowEvalJob" WHERE "group_id" = $1"#;

// _ATTEMPT_COUNTS_SQL: attempts after a leg's stopped_at arrived too late to count against it
pub const ATTEMPT_COUNTS_SQL: &str = r#"SELECT a.job_id, COUNT(*)::bigint AS "attempt_count",
       COALESCE(SUM(a.judge_cost + a.shadow_cost + a.shadow_classifier_cost), 0)::numeric AS "spend"
FROM "LiteLLM_ShadowEvalAttempt" a
JOIN "LiteLLM_ShadowEvalJob" j ON j.id = a.job_id
WHERE a.job_id = ANY($1::text[]) AND (j.stopped_at IS NULL OR a.created_at <= j.stopped_at)
GROUP BY a.job_id"#;

// _STOP_JOB_SQL: the whole state machine, claiming only while a leg still samples inside the window
pub const STOP_JOB_SQL: &str = r#"UPDATE "LiteLLM_ShadowEvalJob"
SET stopped_by = $2, stopped_at = COALESCE(stopped_at, $3::timestamp)
WHERE group_id = $1 AND stopped_by IS NULL
  AND ends_at > (NOW() AT TIME ZONE 'utc')
  AND EXISTS (
    SELECT 1 FROM "LiteLLM_ShadowEvalJob" k
    WHERE k.group_id = $1 AND k.stopped_at IS NULL
      AND (SELECT COUNT(*) FROM "LiteLLM_ShadowEvalAttempt" a WHERE a.job_id = k.id) < k.max_turns
      AND (
        k.max_budget IS NULL
        OR (SELECT COALESCE(SUM(a.judge_cost + a.shadow_cost + a.shadow_classifier_cost), 0) FROM "LiteLLM_ShadowEvalAttempt" a WHERE a.job_id = k.id) < k.max_budget
      )
  )"#;

// one multi-row insert, so a lost partial unique index rejects every leg of the job, never part of it
#[must_use]
pub fn insert_legs(count: usize) -> String {
    let mut statement = String::from(INSERT_LEGS_HEAD);
    let mut parameter = 0_usize;
    for row in 0..count {
        if row > 0 {
            statement.push_str(", ");
        }
        statement.push('(');
        for column in 0..LEG_COLUMNS {
            if column > 0 {
                statement.push_str(", ");
            }
            parameter = parameter.saturating_add(1);
            if column == MAX_BUDGET_SLOT {
                let _written = write!(statement, "${parameter}::numeric::float8");
            } else {
                let _written = write!(statement, "${parameter}");
            }
        }
        statement.push(')');
    }
    statement
}
