use crate::table_ops::access::TableAccess as _;
use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;

use crate::budget_permission_rows::JsonData;
use crate::database_replica::client::DatabaseClient;
use crate::key_repository::table::TokenValue;
use crate::team_rows::team::LiteLLMTeamTable;

use super::access::{CONFIG_ATTRIBUTE, TablePoolAccess};
use super::columns::spend_notification::NotificationColumn;
use super::columns::spend_notification::SpendColumn;
use super::columns::{ConfigColumn, TableColumn, TeamColumn};
use super::rows::{RowDecoder, SpendLogRow, UserNotificationRow};
use super::sql::{self, InsertConflict, Statement};
use super::team_json::{jsonify_object, jsonify_team_object};
use super::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

pub trait TableInserts<Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    fn insert_team(
        &self,
        data: &BTreeMap<TeamColumn, TokenValue>,
    ) -> impl Future<Output = Result<LiteLLMTeamTable, TableOpsError<Client::Error>>> + Send;
    fn insert_config(
        &self,
        data: &[(String, JsonData)],
    ) -> impl Future<Output = Result<(), TableOpsError<Client::Error>>> + Send;
    fn insert_spend(
        &self,
        data: &BTreeMap<SpendColumn, TokenValue>,
    ) -> impl Future<Output = Result<SpendLogRow, TableOpsError<Client::Error>>> + Send;
    fn insert_user_notification(
        &self,
        data: &BTreeMap<NotificationColumn, TokenValue>,
    ) -> impl Future<Output = Result<UserNotificationRow, TableOpsError<Client::Error>>> + Send;
    fn insert_row<Column, Row>(
        &self,
        data: &BTreeMap<Column, TokenValue>,
        jsonify: fn(&BTreeMap<Column, TokenValue>) -> BTreeMap<Column, TokenValue>,
    ) -> impl Future<Output = Result<Row, TableOpsError<Client::Error>>> + Send
    where
        Column: TableColumn + Send + Sync,
        Row: RowDecoder + Send;
}

impl<Client> TableInserts<Client> for TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    async fn insert_team(
        &self,
        data: &BTreeMap<TeamColumn, TokenValue>,
    ) -> Result<LiteLLMTeamTable, TableOpsError<Client::Error>> {
        self.insert_row::<TeamColumn, LiteLLMTeamTable>(data, jsonify_team_object)
            .await
    }

    async fn insert_config(
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

    async fn insert_spend(
        &self,
        data: &BTreeMap<SpendColumn, TokenValue>,
    ) -> Result<SpendLogRow, TableOpsError<Client::Error>> {
        self.insert_row::<SpendColumn, SpendLogRow>(data, jsonify_object::<SpendColumn>)
            .await
    }

    async fn insert_user_notification(
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
                let statement = sql::insert(&payload, InsertConflict::Keep)?;
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
    ClientError: Error + Send + Sync + 'static,
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
    sql::insert(&payload, InsertConflict::Write(&update))
}
