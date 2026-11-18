use std::collections::BTreeMap;

use crate::budget_permission_rows::JsonData;
use crate::database_replica::client::DatabaseClient;
use crate::key_repository::table::TokenValue;
use crate::team_rows::team::LiteLLMTeamTable;

use super::access::{
    CONFIG_ATTRIBUTE, NOTIFICATION_ATTRIBUTE, SPEND_ATTRIBUTE, TEAM_ATTRIBUTE, TablePoolAccess,
};
use super::columns::{ConfigColumn, NotificationColumn, SpendColumn, TeamColumn};
use super::rows::{RowError, SpendLogRow, UserNotificationRow, decode_team_row};
use super::sql::{self, Statement};
use super::team_json::{jsonify_object, jsonify_team_object};
use super::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

impl<Client> TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    pub async fn insert_team(
        &self,
        data: &BTreeMap<TeamColumn, TokenValue>,
    ) -> Result<LiteLLMTeamTable, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::InsertData, || {
            self.insert_team_once(data)
        })
        .await
    }

    pub async fn insert_config(
        &self,
        data: &[(String, JsonData)],
    ) -> Result<(), TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::InsertData, || {
            self.insert_config_once(data)
        })
        .await
    }

    pub async fn insert_spend(
        &self,
        data: &BTreeMap<SpendColumn, TokenValue>,
    ) -> Result<SpendLogRow, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::InsertData, || {
            self.insert_spend_once(data)
        })
        .await
    }

    pub async fn insert_user_notification(
        &self,
        data: &BTreeMap<NotificationColumn, TokenValue>,
    ) -> Result<UserNotificationRow, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::InsertData, || {
            self.insert_user_notification_once(data)
        })
        .await
    }

    async fn insert_team_once(
        &self,
        data: &BTreeMap<TeamColumn, TokenValue>,
    ) -> Result<LiteLLMTeamTable, TableOpsError<Client::Error>> {
        let payload = jsonify_team_object(data);
        let statement = sql::upsert(&payload, None)?;
        let found = self.run_upsert(TEAM_ATTRIBUTE, &statement).await?;
        decode_team_row(&found).map_err(Into::into)
    }

    async fn insert_config_once(
        &self,
        data: &[(String, JsonData)],
    ) -> Result<(), TableOpsError<Client::Error>> {
        for (param_name, value) in data {
            let statement = config_upsert(param_name, value)?;
            self.run_upsert(CONFIG_ATTRIBUTE, &statement).await?;
        }
        for (param_name, _) in data {
            self.ports.invalidator.invalidate_param(param_name).await;
        }
        Ok(())
    }

    async fn insert_spend_once(
        &self,
        data: &BTreeMap<SpendColumn, TokenValue>,
    ) -> Result<SpendLogRow, TableOpsError<Client::Error>> {
        let payload = jsonify_object(data);
        let statement = sql::upsert(&payload, None)?;
        let found = self.run_upsert(SPEND_ATTRIBUTE, &statement).await?;
        let row = serde_json::from_str(&found).map_err(RowError::Model)?;
        Ok(row)
    }

    async fn insert_user_notification_once(
        &self,
        data: &BTreeMap<NotificationColumn, TokenValue>,
    ) -> Result<UserNotificationRow, TableOpsError<Client::Error>> {
        let payload = jsonify_object(data);
        let statement = sql::upsert(&payload, None)?;
        let found = self.run_upsert(NOTIFICATION_ATTRIBUTE, &statement).await?;
        let row = serde_json::from_str(&found).map_err(RowError::Model)?;
        Ok(row)
    }
}

fn config_upsert<ClientError>(
    param_name: &str,
    value: &JsonData,
) -> Result<Statement, TableOpsError<ClientError>>
where
    ClientError: std::error::Error + Send + Sync + 'static,
{
    let encoded = serde_json::to_string(value).map_err(|source| TableOpsError::Encode {
        column: "param_value",
        source,
    })?;
    let stored = TokenValue::Json(JsonData::String(encoded));
    let payload: BTreeMap<ConfigColumn, TokenValue> = BTreeMap::from([
        (
            ConfigColumn::ParamName,
            TokenValue::Text(param_name.to_owned()),
        ),
        (ConfigColumn::ParamValue, stored.clone()),
    ]);
    let update: BTreeMap<ConfigColumn, TokenValue> =
        BTreeMap::from([(ConfigColumn::ParamValue, stored)]);
    sql::upsert(&payload, Some(&update))
}
