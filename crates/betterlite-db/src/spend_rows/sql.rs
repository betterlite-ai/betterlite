use deadpool_postgres::GenericClient as _;
use rust_decimal::Decimal;
use thiserror::Error;
use tokio_postgres::types::ToSql;

use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::PostgresStore;

use super::rows::{SpendCounterRows, SpendRow, SpendRowLookup};

const KEY_SPEND_SQL: &str =
    r#"SELECT spend::numeric FROM "LiteLLM_VerificationToken" WHERE token = $1"#;
const TEAM_MEMBER_SPEND_SQL: &str =
    r#"SELECT spend::numeric FROM "LiteLLM_TeamMembership" WHERE user_id = $1 AND team_id = $2"#;
const TEAM_SPEND_SQL: &str = r#"SELECT spend::numeric FROM "LiteLLM_TeamTable" WHERE team_id = $1"#;
const USER_SPEND_SQL: &str = r#"SELECT spend::numeric FROM "LiteLLM_UserTable" WHERE user_id = $1"#;
const ORGANIZATION_SPEND_SQL: &str =
    r#"SELECT spend::numeric FROM "LiteLLM_OrganizationTable" WHERE organization_id = $1"#;
const PROJECT_SPEND_SQL: &str =
    r#"SELECT spend::numeric FROM "LiteLLM_ProjectTable" WHERE project_id = $1"#;
const END_USER_SPEND_SQL: &str =
    r#"SELECT spend::numeric FROM "LiteLLM_EndUserTable" WHERE user_id = $1"#;

#[derive(Debug, Error)]
pub enum SpendRowsError {
    #[error(transparent)]
    Database(#[from] DbError),
}

// The spend columns are DOUBLE PRECISION, so the read casts to numeric and rust_decimal keeps the exact stored value
pub struct SqlSpendCounterRows {
    store: PostgresStore,
}

impl From<PostgresStore> for SqlSpendCounterRows {
    fn from(store: PostgresStore) -> Self {
        Self { store }
    }
}

impl SpendCounterRows for SqlSpendCounterRows {
    type Error = SpendRowsError;

    async fn spend_row(
        &self,
        lookup: SpendRowLookup<'_>,
    ) -> Result<Option<SpendRow>, SpendRowsError> {
        let store = &self.store;
        match lookup {
            SpendRowLookup::Key(token) => stored_spend_row(store, KEY_SPEND_SQL, &[&token]).await,
            SpendRowLookup::TeamMember { user_id, team_id } => {
                stored_spend_row(store, TEAM_MEMBER_SPEND_SQL, &[&user_id, &team_id]).await
            }
            SpendRowLookup::Team(team_id) => {
                stored_spend_row(store, TEAM_SPEND_SQL, &[&team_id]).await
            }
            SpendRowLookup::User(user_id) => {
                stored_spend_row(store, USER_SPEND_SQL, &[&user_id]).await
            }
            SpendRowLookup::Organization(organization_id) => {
                stored_spend_row(store, ORGANIZATION_SPEND_SQL, &[&organization_id]).await
            }
            SpendRowLookup::Project(project_id) => {
                stored_spend_row(store, PROJECT_SPEND_SQL, &[&project_id]).await
            }
            SpendRowLookup::EndUser(user_id) => {
                stored_spend_row(store, END_USER_SPEND_SQL, &[&user_id]).await
            }
        }
    }
}

async fn stored_spend_row(
    store: &PostgresStore,
    statement: &str,
    parameters: &[&(dyn ToSql + Sync)],
) -> Result<Option<SpendRow>, SpendRowsError> {
    let client = store.session().await?;
    let found = client
        .query_opt(statement, parameters)
        .await
        .map_err(database)?;
    let Some(row) = found else {
        return Ok(None);
    };
    let spend: Option<Decimal> = row.try_get(0).map_err(database)?;
    Ok(Some(SpendRow { spend }))
}

fn database(source: tokio_postgres::Error) -> SpendRowsError {
    SpendRowsError::Database(backend(source))
}
