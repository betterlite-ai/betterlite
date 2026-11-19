use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use thiserror::Error;
use tokio_postgres::types::ToSql;

use super::rows::{
    SpendAggregateRow, SpendCounterRows, SpendRow, SpendRowLookup, WindowEntity, WindowSpendRow,
};

// the tables and unique keys each repository B722/B724/B726/B723 owns, read as raw stored rows
const KEY_SPEND_SQL: &str =
    r#"SELECT to_jsonb(t.*)::text FROM "LiteLLM_VerificationToken" t WHERE t.token = $1"#;
const TEAM_MEMBER_SPEND_SQL: &str = r#"SELECT to_jsonb(t.*)::text FROM "LiteLLM_TeamMembership" t
WHERE t.user_id = $1 AND t.team_id = $2"#;
const TEAM_SPEND_SQL: &str =
    r#"SELECT to_jsonb(t.*)::text FROM "LiteLLM_TeamTable" t WHERE t.team_id = $1"#;
const USER_SPEND_SQL: &str =
    r#"SELECT to_jsonb(t.*)::text FROM "LiteLLM_UserTable" t WHERE t.user_id = $1"#;
const ORGANIZATION_SPEND_SQL: &str = r#"SELECT to_jsonb(t.*)::text
FROM "LiteLLM_OrganizationTable" t WHERE t.organization_id = $1"#;
const PROJECT_SPEND_SQL: &str =
    r#"SELECT to_jsonb(t.*)::text FROM "LiteLLM_ProjectTable" t WHERE t.project_id = $1"#;
const END_USER_SPEND_SQL: &str =
    r#"SELECT to_jsonb(t.*)::text FROM "LiteLLM_EndUserTable" t WHERE t.user_id = $1"#;
const WINDOW_SPEND_SQL: &str = r#"SELECT to_jsonb(t.*)::text FROM "LiteLLM_BudgetWindowSpend" t
WHERE t.entity_type = $1 AND t.entity_id = $2 AND t.window_duration = $3"#;
const KEY_SPEND_LOGS_SQL: &str = r#"SELECT to_jsonb(g)::text FROM (
    SELECT api_key, SUM(spend) AS spend FROM "LiteLLM_SpendLogs"
    WHERE api_key = $1 AND "startTime" >= $2 GROUP BY api_key
) g"#;
const TEAM_SPEND_LOGS_SQL: &str = r#"SELECT to_jsonb(g)::text FROM (
    SELECT team_id, SUM(spend) AS spend FROM "LiteLLM_SpendLogs"
    WHERE team_id = $1 AND "startTime" >= $2 GROUP BY team_id
) g"#;

#[derive(Debug, Error)]
pub enum SpendRowsError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("the stored spend row is not readable JSON")]
    Row(#[source] serde_json::Error),
}

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
            SpendRowLookup::Key(token) => json_row(store, KEY_SPEND_SQL, &[&token]).await,
            SpendRowLookup::TeamMember { user_id, team_id } => {
                json_row(store, TEAM_MEMBER_SPEND_SQL, &[&user_id, &team_id]).await
            }
            SpendRowLookup::Team(team_id) => json_row(store, TEAM_SPEND_SQL, &[&team_id]).await,
            SpendRowLookup::User(user_id) => json_row(store, USER_SPEND_SQL, &[&user_id]).await,
            SpendRowLookup::Organization(organization_id) => {
                json_row(store, ORGANIZATION_SPEND_SQL, &[&organization_id]).await
            }
            SpendRowLookup::Project(project_id) => {
                json_row(store, PROJECT_SPEND_SQL, &[&project_id]).await
            }
            SpendRowLookup::EndUser(user_id) => {
                json_row(store, END_USER_SPEND_SQL, &[&user_id]).await
            }
        }
    }

    async fn window_row(
        &self,
        entity: WindowEntity,
        entity_id: &str,
        window_duration: &str,
    ) -> Result<Option<WindowSpendRow>, SpendRowsError> {
        let label = entity.table_label();
        let parameters: [&(dyn ToSql + Sync); 3] = [&label, &entity_id, &window_duration];
        json_row(&self.store, WINDOW_SPEND_SQL, &parameters).await
    }

    async fn window_log_rows(
        &self,
        entity: WindowEntity,
        entity_id: &str,
        window_start: DateTime<Utc>,
    ) -> Result<Vec<SpendAggregateRow>, SpendRowsError> {
        let statement = match entity {
            WindowEntity::Key => KEY_SPEND_LOGS_SQL,
            WindowEntity::Team => TEAM_SPEND_LOGS_SQL,
        };
        // Prisma's DateTime lands in a timestamp(3) column, so the bound start is the UTC wall clock
        let starts = window_start.naive_utc();
        let sum: Option<SpendSumRow> =
            json_row(&self.store, statement, &[&entity_id, &starts]).await?;
        let Some(row) = sum else {
            return Ok(Vec::new());
        };
        let spend = row.spend.unwrap_or(JsonData::Null);
        let total = JsonData::Object(BTreeMap::from([("spend".to_owned(), spend)]));
        Ok(vec![SpendAggregateRow::Attribute { sum: Some(total) }])
    }
}

#[derive(Deserialize)]
struct SpendSumRow {
    spend: Option<JsonData>,
}

async fn json_row<Record>(
    store: &PostgresStore,
    statement: &str,
    parameters: &[&(dyn ToSql + Sync)],
) -> Result<Option<Record>, SpendRowsError>
where
    Record: DeserializeOwned,
{
    let client = store.session().await.map_err(SpendRowsError::Database)?;
    let query = client.query_opt(statement, parameters);
    let found = query
        .await
        .map_err(|source| SpendRowsError::Database(backend(source)))?;
    let Some(row) = found else {
        return Ok(None);
    };
    let payload: String = row
        .try_get(0)
        .map_err(|source| SpendRowsError::Database(backend(source)))?;
    serde_json::from_str(&payload)
        .map(Some)
        .map_err(SpendRowsError::Row)
}
