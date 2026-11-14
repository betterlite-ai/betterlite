pub(super) const LENSES: &str = r#"SELECT data::text AS data FROM "LiteLLM_Lens" ORDER BY id"#;
pub(super) const LENS: &str = r#"SELECT data::text AS data FROM "LiteLLM_Lens" WHERE id=$1"#;
pub(super) const CREATE_LENS: &str =
    r#"INSERT INTO "LiteLLM_Lens" (id, version, data) VALUES ($1,0,$2::jsonb)"#;
pub(super) const UPDATE_LENS: &str = r#"WITH previous AS MATERIALIZED (
    SELECT data FROM "LiteLLM_Lens" WHERE id=$2 AND version=$3 FOR UPDATE
), updated AS (
    UPDATE "LiteLLM_Lens" SET data=$1::jsonb, version=version+1
    WHERE id=$2 AND version=$3 AND EXISTS (SELECT 1 FROM previous) RETURNING id
), archived AS (
    INSERT INTO "LiteLLM_LensRun" (id, lens_id, created_at, data)
    SELECT job->>'id', $2, (job->>'created_at')::timestamp, job
    FROM previous, jsonb_array_elements(previous.data->'jobs') AS job
    WHERE EXISTS (SELECT 1 FROM updated)
      AND NOT EXISTS (
          SELECT 1 FROM jsonb_array_elements(($1::jsonb)->'jobs') AS retained
          WHERE retained->>'id'=job->>'id'
      )
    ON CONFLICT (id) DO NOTHING
)
SELECT to_jsonb(count(*))::text AS data FROM updated"#;
pub(super) const JOBS: &str = r#"SELECT data::text AS data FROM (
    SELECT data FROM "LiteLLM_LensRun" WHERE lens_id=$1
    UNION ALL
    SELECT jsonb_array_elements(data->'jobs') AS data FROM "LiteLLM_Lens" WHERE id=$1
) AS jobs ORDER BY data->>'created_at' DESC, data->>'id' DESC LIMIT 50 OFFSET $2"#;
pub(super) const JOB: &str = r#"SELECT data::text AS data FROM "LiteLLM_LensRun" WHERE lens_id=$1 AND id=$2
UNION ALL SELECT job::text AS data FROM "LiteLLM_Lens", jsonb_array_elements(data->'jobs') AS job
WHERE id=$1 AND job->>'id'=$2 LIMIT 1"#;
pub(super) const WORKERS: &str = r#"SELECT data::text AS data FROM "LiteLLM_LensWorker""#;
pub(super) const WORKER: &str =
    r#"SELECT data::text AS data FROM "LiteLLM_LensWorker" WHERE token_hash=$1"#;
pub(super) const INSERT_WORKER: &str =
    r#"INSERT INTO "LiteLLM_LensWorker" (id,token_hash,data) VALUES ($1,$2,$3::jsonb)"#;
pub(super) const UPDATE_WORKER: &str =
    r#"UPDATE "LiteLLM_LensWorker" SET data=$1::jsonb WHERE id=$2"#;
pub(super) const SET_WORKER_BILLING: &str = r#"UPDATE "LiteLLM_LensWorker"
SET data=jsonb_set(data, '{analysis_key_id}', to_jsonb($1::text))
WHERE id=$2 AND COALESCE((data->>'revoked')::boolean, false)=false RETURNING data::text AS data"#;
pub(super) const REVOKE_WORKER: &str =
    r#"UPDATE "LiteLLM_LensWorker" SET data=jsonb_set(data, '{revoked}', 'true') WHERE id=$1"#;
pub(super) const HEARTBEAT: &str = r#"UPDATE "LiteLLM_LensWorker"
SET data=jsonb_set(data, '{last_seen}', to_jsonb($1::text)) WHERE id=$2"#;
