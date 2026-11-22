use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_repository::decoder::decode_team_record;
use betterlite_db::team_repository::repository::{TeamDatabase, TeamRepository};
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use tokio_postgres::Row;
use tokio_postgres::types::Json;

use super::error::VisibilityError;
use super::owners::OwnerPair;
use super::query::{OWNERS_SQL, PayloadQuery};
use super::store::{PayloadRow, VisibilityStore};
use crate::management_admins::adapters::PostgresTeamReads;
use crate::management_admins::ports::TeamReads as _;

pub struct PostgresVisibility {
    pub store: PostgresStore,
}

#[async_trait]
impl VisibilityStore for PostgresVisibility {
    async fn owners(&self, lookup_id: &str) -> Result<Vec<OwnerPair>, VisibilityError> {
        let client = self.store.session().await?;
        let rows = client.query(OWNERS_SQL, &[&lookup_id]).await?;
        let mut owners = Vec::with_capacity(rows.len());
        for row in &rows {
            owners.push(OwnerPair {
                user: row.try_get("user")?,
                team_id: row.try_get("team_id")?,
            });
        }
        Ok(owners)
    }

    async fn payload_rows(
        &self,
        query: &PayloadQuery<'_>,
    ) -> Result<Vec<PayloadRow>, VisibilityError> {
        let client = self.store.session().await?;
        let rows = client.query(query.sql, &query.params()).await?;
        let mut payloads = Vec::with_capacity(rows.len());
        for row in &rows {
            payloads.push(payload_row(row)?);
        }
        Ok(payloads)
    }

    async fn team(&self, team_id: &str) -> Result<Option<LiteLLMTeamTable>, VisibilityError> {
        let repository = TeamRepository {
            database: TeamDatabase {
                db: &self.store,
                writer_db: &self.store,
            },
            use_writer: false,
        };
        repository
            .find_by_id(team_id)
            .await
            .map_err(|error| VisibilityError::Team(Box::new(error)))
    }

    async fn teams(&self, team_ids: &[String]) -> Result<Vec<LiteLLMTeamTable>, VisibilityError> {
        let reads = PostgresTeamReads {
            store: self.store.clone(),
        };
        let records = reads
            .teams_by_ids(team_ids)
            .await
            .map_err(|error| VisibilityError::TeamList(Box::new(error)))?;
        let mut teams = Vec::with_capacity(records.len());
        for record in &records {
            let decoded = decode_team_record(Some(record))
                .map_err(|error| VisibilityError::TeamRecord(Box::new(error)))?;
            teams.extend(decoded);
        }
        Ok(teams)
    }
}

fn payload_row(row: &Row) -> Result<PayloadRow, tokio_postgres::Error> {
    let messages: Option<Json<JsonData>> = row.try_get("messages")?;
    let response: Option<Json<JsonData>> = row.try_get("response")?;
    let proxy_server_request: Option<Json<JsonData>> = row.try_get("proxy_server_request")?;
    let metadata: Option<Json<JsonData>> = row.try_get("metadata")?;
    Ok(PayloadRow {
        request_id: row.try_get("request_id")?,
        messages: messages.map(|Json(data)| data),
        response: response.map(|Json(data)| data),
        proxy_server_request: proxy_server_request.map(|Json(data)| data),
        metadata: metadata.map(|Json(data)| data),
        user: row.try_get("user")?,
        team_id: row.try_get("team_id")?,
    })
}
