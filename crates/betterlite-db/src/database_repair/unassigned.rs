use tokio_postgres::Client;
use tokio_postgres::Error as PostgresError;

pub const PRODUCTION_WARNING: &str =
    "DO NOT run this in Production....Running update_unassigned_teams";

const UPDATE_SQL: &str = r#"UPDATE "LiteLLM_SpendLogs"
SET team_id = (
    SELECT vt.team_id
    FROM "LiteLLM_VerificationToken" vt
    WHERE vt.token = "LiteLLM_SpendLogs".api_key
)
WHERE team_id IS NULL
AND EXISTS (
    SELECT 1
    FROM "LiteLLM_VerificationToken" vt
    WHERE vt.token = "LiteLLM_SpendLogs".api_key
)"#;

#[derive(Debug, thiserror::Error)]
pub enum UnassignedTeamError {
    #[error("Error apply_db_fixes: {source}")]
    Update {
        #[source]
        source: PostgresError,
    },
}

pub async fn apply_db_fixes(client: &Client) -> Result<u64, UnassignedTeamError> {
    client
        .execute(UPDATE_SQL, &[])
        .await
        .map_err(|source| UnassignedTeamError::Update { source })
}
