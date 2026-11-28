use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use tokio_postgres::types::ToSql;

use crate::team_fields::tables::{raw_team_table, team_table};
use crate::team_list_query::TeamListQueryError;
use crate::team_list_query::read::find_teams;

mod tests;

const TEAM_COUNT_SQL: &str = r#"SELECT count(*) FROM "LiteLLM_TeamTable""#;
const TEAM_PAGE_SQL: &str = r#"SELECT row_to_json(t)::text FROM "LiteLLM_TeamTable" t ORDER BY t."team_alias" ASC OFFSET $1 LIMIT $2"#;

#[derive(Debug, thiserror::Error)]
pub enum TeamPageError {
    #[error("team page {page} with page size {page_size} has no database offset")]
    Skip { page: u64, page_size: u64 },
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Page(#[from] TeamListQueryError),
}

// LiteLLM's get_paginated_teams: one page of teams and the whole table's count
pub async fn get_paginated_teams(
    client: PrismaClient<'_>,
    page_size: u64,
    page: u64,
) -> (Vec<LiteLLMTeamTable>, u64) {
    match read_page(client, page_size, page).await {
        Ok((teams, total)) => (teams, total),
        Err(error) => {
            tracing::error!("[Non-Blocking] Error getting paginated teams: {error}");
            (Vec::new(), 0)
        }
    }
}

async fn read_page(
    client: PrismaClient<'_>,
    page_size: u64,
    page: u64,
) -> Result<(Vec<LiteLLMTeamTable>, u64), TeamPageError> {
    let previous = page
        .checked_sub(1)
        .ok_or(TeamPageError::Skip { page, page_size })?;
    let skipped = previous
        .checked_mul(page_size)
        .ok_or(TeamPageError::Skip { page, page_size })?;
    let Ok(offset) = i64::try_from(skipped) else {
        return Err(TeamPageError::Skip { page, page_size });
    };
    let Ok(limit) = i64::try_from(page_size) else {
        return Err(TeamPageError::Skip { page, page_size });
    };
    // Prisma counts and pages in two statements, so a concurrent write can make them disagree
    let total = team_count(client).await?;
    let parameters: [&(dyn ToSql + Sync); 2] = [&offset, &limit];
    let teams = find_teams(Some(client), TEAM_PAGE_SQL, &parameters).await?;
    Ok((teams, total))
}

async fn team_count(client: PrismaClient<'_>) -> Result<u64, TeamPageError> {
    let repository = team_table(Some(client))?;
    let store = raw_team_table(&repository).store;
    let session = store.session().await?;
    let row = session
        .query_one(TEAM_COUNT_SQL, &[])
        .await
        .map_err(backend)?;
    let stored: i64 = row.try_get(0).map_err(backend)?;
    let total = u64::try_from(stored).map_err(|source| TeamListQueryError::Count { source })?;
    Ok(total)
}
