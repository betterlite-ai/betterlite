use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use tokio_postgres::types::ToSql;

// find_many with the litellm_budget_table relation included, as the pinned helper asks for it
const MEMBERSHIPS_SQL: &str = r#"SELECT (to_jsonb(entry) || jsonb_build_object('litellm_budget_table', (SELECT to_jsonb(budget) FROM "LiteLLM_BudgetTable" AS budget WHERE budget."budget_id" = entry."budget_id")))::text FROM "LiteLLM_TeamMembership" AS entry WHERE entry."team_id" = ANY($1::text[]) AND ($2::text IS NULL OR entry."user_id" = $2::text) ORDER BY entry."team_id", entry."user_id""#;

#[derive(Debug, thiserror::Error)]
pub enum MembershipError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("a stored team membership row cannot be decoded")]
    Decode(#[from] serde_json::Error),
}

pub async fn all_team_memberships(
    store: &PostgresStore,
    team_ids: &[String],
    user_id: Option<&str>,
) -> Result<Vec<LiteLLMTeamMembership>, MembershipError> {
    let client = store.session().await?;
    let parameters: [&(dyn ToSql + Sync); 2] = [&team_ids, &user_id];
    let rows = client
        .query(MEMBERSHIPS_SQL, &parameters)
        .await
        .map_err(backend)?;
    let mut memberships = Vec::with_capacity(rows.len());
    for row in &rows {
        let text: String = row.try_get(0).map_err(backend)?;
        memberships.push(serde_json::from_str(&text).map_err(MembershipError::from)?);
    }
    Ok(memberships)
}
