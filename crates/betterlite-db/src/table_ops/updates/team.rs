use std::collections::BTreeMap;

use crate::database_replica::client::DatabaseClient;
use crate::key_repository::table::TokenValue;

use crate::table_ops::access::{TEAM_ATTRIBUTE, TablePoolAccess};
use crate::table_ops::columns::TeamColumn;
use crate::table_ops::rows::decode_team_row;
use crate::table_ops::sql;
use crate::table_ops::team_json::jsonify_team_update;
use crate::table_ops::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

use super::TeamUpdateResult;

impl<Client> TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    pub async fn update_team(
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
        let update = match update_key_values {
            Some(values) => jsonify_team_update(values),
            None => create.clone(),
        };
        let statement = sql::upsert(&create, Some(&update))?;
        let found = self.run_upsert(TEAM_ATTRIBUTE, &statement).await?;
        let data = decode_team_row(&found).map_err(Into::into)?;
        Ok(TeamUpdateResult {
            team_id: resolved,
            data,
        })
    }
}

// the team branch falls back to the row's own team_id when the caller names none
fn resolve_team_id<ClientError>(
    team_id: Option<&str>,
    data: &BTreeMap<TeamColumn, TokenValue>,
) -> Result<String, TableOpsError<ClientError>>
where
    ClientError: std::error::Error + Send + Sync + 'static,
{
    if let Some(team_id) = team_id {
        return Ok(team_id.to_owned());
    }
    match data.get(&TeamColumn::TeamId) {
        Some(TokenValue::Text(team_id)) => Ok(team_id.clone()),
        Some(_) => Err(TableOpsError::WrongType { column: "team_id" }),
        None => Err(TableOpsError::MissingField { field: "team_id" }),
    }
}
