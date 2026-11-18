use std::collections::BTreeMap;

use crate::budget_permission_rows::JsonData;
use crate::database_replica::client::DatabaseClient;
use crate::key_repository::table::TokenValue;
use crate::team_rows::team::LiteLLMTeamTable;

use super::access::{
    CONFIG_ATTRIBUTE, NOTIFICATION_ATTRIBUTE, SPEND_ATTRIBUTE, TEAM_ATTRIBUTE, TablePoolAccess,
};
use super::columns::{ConfigColumn, NotificationColumn, SpendColumn, TableColumn, TeamColumn};
use super::rows::{RowDecoder, SpendLogRow, UserNotificationRow};
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
        self.insert_row::<TeamColumn, LiteLLMTeamTable>(data, jsonify_team_object)
            .await
    }

    pub async fn insert_config(
        &self,
        data: &[(String, JsonData)],
    ) -> Result<(), TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::InsertData, || async {
            for (param_name, value) in data {
                let statement = config_upsert(param_name, value)?;
                self.run_upsert(CONFIG_ATTRIBUTE, &statement).await?;
            }
            for (param_name, _) in data {
                self.ports.invalidator.invalidate_param(param_name).await;
            }
            Ok(())
        })
        .await
    }

    pub async fn insert_spend(
        &self,
        data: &BTreeMap<SpendColumn, TokenValue>,
    ) -> Result<SpendLogRow, TableOpsError<Client::Error>> {
        self.insert_row::<SpendColumn, SpendLogRow>(data, jsonify_object::<SpendColumn>)
            .await
    }

    pub async fn insert_user_notification(
        &self,
        data: &BTreeMap<NotificationColumn, TokenValue>,
    ) -> Result<UserNotificationRow, TableOpsError<Client::Error>> {
        self.insert_row::<NotificationColumn, UserNotificationRow>(
            data,
            jsonify_object::<NotificationColumn>,
        )
        .await
    }

    async fn insert_row<Column, Row>(
        &self,
        data: &BTreeMap<Column, TokenValue>,
        jsonify: fn(&BTreeMap<Column, TokenValue>) -> BTreeMap<Column, TokenValue>,
    ) -> Result<Row, TableOpsError<Client::Error>>
    where
        Column: TableColumn + Send + Sync,
        Row: RowDecoder + Send,
    {
        let text = self
            .attempt_with(DATA_RETRY, OperationCallType::InsertData, || async {
                let payload = jsonify(data);
                let statement = sql::upsert(&payload, None)?;
                self.run_upsert(Column::ATTRIBUTE, &statement).await
            })
            .await?;
        Ok(Row::decode_row(&text)?)
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
