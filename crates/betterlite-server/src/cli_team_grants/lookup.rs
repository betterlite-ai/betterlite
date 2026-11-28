use std::convert::Infallible;

use betterlite_auth::identity_grants::team_grants::TeamColumnsError;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_rows::aliases::ModelAliases;

use super::records::{CliSsoTeamDetail, TeamGrantRow, cli_sso_team_detail};

mod tests;

const TEAM_DETAILS_SQL: &str = r#"SELECT t.team_id, t.team_alias, t.models, m.aliases FROM "LiteLLM_TeamTable" t LEFT JOIN "LiteLLM_ModelTable" m ON m.id = t.model_id WHERE t.team_id = ANY($1)"#;

#[derive(Debug, thiserror::Error)]
enum LookupError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    AliasColumn(#[from] serde_json::Error),
    #[error(transparent)]
    Columns(#[from] TeamColumnsError),
}

async fn read_team_details(
    store: &PostgresStore,
    teams: &[String],
) -> Result<Vec<CliSsoTeamDetail>, LookupError> {
    let client = store.session().await?;
    let rows = client
        .query(TEAM_DETAILS_SQL, &[&teams])
        .await
        .map_err(backend::<Infallible>)?;
    let mut details = Vec::with_capacity(rows.len());
    for row in &rows {
        let team_id: String = row.try_get(0).map_err(backend::<Infallible>)?;
        let team_alias: Option<String> = row.try_get(1).map_err(backend::<Infallible>)?;
        let models: Vec<String> = row.try_get(2).map_err(backend::<Infallible>)?;
        // Prisma reads a JSON null in the aliases column as no alias table, like a SQL null
        let aliases: Option<JsonValue> = row.try_get(3).map_err(backend::<Infallible>)?;
        let model_aliases = match aliases {
            None | Some(JsonValue::Null) => None,
            Some(value) => Some(serde_json::from_value::<ModelAliases>(value)?),
        };
        details.push(cli_sso_team_detail(TeamGrantRow {
            team_id,
            team_alias,
            models,
            model_aliases,
        })?);
    }
    Ok(details)
}

// None is the lookup failing, which every caller refuses instead of reading it as no teams
#[must_use]
pub async fn fetch_cli_sso_team_details(
    store: &PostgresStore,
    teams: &[String],
) -> Option<Vec<CliSsoTeamDetail>> {
    if teams.is_empty() {
        return Some(Vec::new());
    }
    match read_team_details(store, teams).await {
        Ok(details) => Some(details),
        Err(error) => {
            tracing::error!("Error fetching team details for CLI SSO session: {error}");
            None
        }
    }
}
