use crate::table_ops::access::TableAccess as _;
use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;

use tokio_postgres::types::ToSql;

use crate::budget_permission_rows::Timestamp;
use crate::database_replica::client::DatabaseClient;
use crate::key_repository::table::TokenValue;
use crate::team_rows::team::LiteLLMTeamTable;

use crate::table_ops::access::{TEAM_ATTRIBUTE, TablePoolAccess};
use crate::table_ops::columns::TeamColumn;
use crate::table_ops::rows::RowDecoder as _;
use crate::table_ops::sql::{self, Bound, InsertConflict};
use crate::table_ops::team_json::jsonify_team_update;
use crate::table_ops::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

use super::TeamUpdateResult;
use super::stamp_updated_at;

pub trait TeamUpdates<Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    fn update_team(
        &self,
        team_id: Option<&str>,
        data: &BTreeMap<TeamColumn, TokenValue>,
        update_key_values: Option<&BTreeMap<TeamColumn, TokenValue>>,
    ) -> impl Future<Output = Result<TeamUpdateResult, TableOpsError<Client::Error>>> + Send;
    fn update_team_once(
        &self,
        team_id: Option<&str>,
        data: &BTreeMap<TeamColumn, TokenValue>,
        update_key_values: Option<&BTreeMap<TeamColumn, TokenValue>>,
    ) -> impl Future<Output = Result<TeamUpdateResult, TableOpsError<Client::Error>>> + Send;
}

impl<Client> TeamUpdates<Client> for TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    async fn update_team(
        &self,
        team_id: Option<&str>,
        data: &BTreeMap<TeamColumn, TokenValue>,
        update_key_values: Option<&BTreeMap<TeamColumn, TokenValue>>,
    ) -> Result<TeamUpdateResult, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::UpdateData, || {
            self.update_team_once(team_id, data, update_key_values)
        })
        .await
    }

    async fn update_team_once(
        &self,
        team_id: Option<&str>,
        data: &BTreeMap<TeamColumn, TokenValue>,
        update_key_values: Option<&BTreeMap<TeamColumn, TokenValue>>,
    ) -> Result<TeamUpdateResult, TableOpsError<Client::Error>> {
        let resolved = resolve_team_id(team_id, data)?;
        let mut create = jsonify_team_update(data);
        if !data.contains_key(&TeamColumn::TeamId) {
            create.insert(TeamColumn::TeamId, TokenValue::Text(resolved.clone()));
        }
        let mut update = update_key_values.map_or_else(|| create.clone(), jsonify_team_update);
        let now = Timestamp::Naive(self.ports.clock.now().naive_utc());
        stamp_updated_at(&mut create, now);
        stamp_updated_at(&mut update, now);
        // Prisma's upsert filters on the resolved id, so a different team_id in the payload only reaches the written row
        let statement = sql::update_where(&update, &resolved)?;
        let parameters: Vec<&(dyn ToSql + Sync)> = statement
            .parameters
            .values
            .iter()
            .map(Bound::as_sql)
            .collect();
        let stored = self
            .text_rows(TEAM_ATTRIBUTE, "update", &statement.sql, &parameters)
            .await?
            .into_iter()
            .next();
        let found = if let Some(text) = stored {
            text
        } else {
            let insert = sql::insert(&create, InsertConflict::Reject)?;
            self.run_upsert(TEAM_ATTRIBUTE, &insert).await?
        };
        let decoded = LiteLLMTeamTable::decode_row(&found)?;
        Ok(TeamUpdateResult {
            team_id: resolved,
            data: decoded,
        })
    }
}

// the team branch falls back to the row's own team_id when the caller names none
fn resolve_team_id<ClientError>(
    team_id: Option<&str>,
    data: &BTreeMap<TeamColumn, TokenValue>,
) -> Result<String, TableOpsError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
{
    if let Some(explicit) = team_id {
        return Ok(explicit.to_owned());
    }
    match data.get(&TeamColumn::TeamId) {
        Some(TokenValue::Text(stored_id)) => Ok(stored_id.clone()),
        Some(_) => Err(TableOpsError::WrongType { column: "team_id" }),
        None => Err(TableOpsError::MissingField { field: "team_id" }),
    }
}
