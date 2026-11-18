use crate::table_ops::access::TableAccess as _;
use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;

use secrecy::ExposeSecret as _;

use crate::budget_permission_rows::Timestamp;
use crate::database_replica::client::DatabaseClient;
use crate::key_repository::table::TokenValue;
use crate::key_repository::table::columns::TokenColumn;
use crate::key_user_writes::columns::UserColumn;
use crate::key_user_writes::hash::hash_token_if_needed;

use crate::table_ops::access::TablePoolAccess;
use crate::table_ops::columns::budget_enduser::BudgetColumn;
use crate::table_ops::columns::budget_enduser::EndUserColumn;
use crate::table_ops::columns::{TableColumn, TeamColumn};
use crate::table_ops::sql::{self, InsertConflict};
use crate::table_ops::team_json::{jsonify_object, jsonify_team_object};
use crate::table_ops::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

use super::stamp_updated_at;

pub trait BatchUpdates<Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    fn update_many_keys(
        &self,
        rows: &[BTreeMap<TokenColumn, TokenValue>],
    ) -> impl Future<Output = Result<(), TableOpsError<Client::Error>>> + Send;
    fn update_many_users(
        &self,
        rows: &[BTreeMap<UserColumn, TokenValue>],
    ) -> impl Future<Output = Result<(), TableOpsError<Client::Error>>> + Send;
    fn update_many_end_users(
        &self,
        rows: &[BTreeMap<EndUserColumn, TokenValue>],
    ) -> impl Future<Output = Result<(), TableOpsError<Client::Error>>> + Send;
    fn update_many_budgets(
        &self,
        rows: &[BTreeMap<BudgetColumn, TokenValue>],
    ) -> impl Future<Output = Result<(), TableOpsError<Client::Error>>> + Send;
    fn update_many_teams(
        &self,
        rows: &[BTreeMap<TeamColumn, TokenValue>],
    ) -> impl Future<Output = Result<(), TableOpsError<Client::Error>>> + Send;
    fn update_many_object_rows<Column>(
        &self,
        rows: &[BTreeMap<Column, TokenValue>],
        jsonify: fn(&BTreeMap<Column, TokenValue>) -> BTreeMap<Column, TokenValue>,
    ) -> impl Future<Output = Result<(), TableOpsError<Client::Error>>> + Send
    where
        Column: TableColumn + Send + Sync;
}

impl<Client> BatchUpdates<Client> for TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    async fn update_many_keys(
        &self,
        rows: &[BTreeMap<TokenColumn, TokenValue>],
    ) -> Result<(), TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::UpdateData, || async {
            let mut statements = Vec::new();
            for row in rows {
                let mut payload = hash_key_row(&jsonify_object(row));
                stamp_updated_at(
                    &mut payload,
                    Timestamp::Naive(self.ports.clock.now().naive_utc()),
                );
                statements.push(sql::update_where(&payload, &token_key(&payload)?)?);
            }
            self.execute_statements(TokenColumn::ATTRIBUTE, &statements)
                .await
        })
        .await
    }

    async fn update_many_users(
        &self,
        rows: &[BTreeMap<UserColumn, TokenValue>],
    ) -> Result<(), TableOpsError<Client::Error>> {
        self.update_many_object_rows(rows, jsonify_object::<UserColumn>)
            .await
    }

    async fn update_many_end_users(
        &self,
        rows: &[BTreeMap<EndUserColumn, TokenValue>],
    ) -> Result<(), TableOpsError<Client::Error>> {
        self.update_many_object_rows(rows, jsonify_object::<EndUserColumn>)
            .await
    }

    async fn update_many_budgets(
        &self,
        rows: &[BTreeMap<BudgetColumn, TokenValue>],
    ) -> Result<(), TableOpsError<Client::Error>> {
        self.update_many_object_rows(rows, jsonify_object::<BudgetColumn>)
            .await
    }

    async fn update_many_teams(
        &self,
        rows: &[BTreeMap<TeamColumn, TokenValue>],
    ) -> Result<(), TableOpsError<Client::Error>> {
        self.update_many_object_rows(rows, jsonify_team_object)
            .await
    }

    async fn update_many_object_rows<Column>(
        &self,
        rows: &[BTreeMap<Column, TokenValue>],
        jsonify: fn(&BTreeMap<Column, TokenValue>) -> BTreeMap<Column, TokenValue>,
    ) -> Result<(), TableOpsError<Client::Error>>
    where
        Column: TableColumn + Send + Sync,
    {
        self.attempt_with(DATA_RETRY, OperationCallType::UpdateData, || async {
            let mut statements = Vec::new();
            for row in rows {
                let mut payload = jsonify(&without_nulls(row));
                stamp_updated_at(
                    &mut payload,
                    Timestamp::Naive(self.ports.clock.now().naive_utc()),
                );
                statements.push(sql::insert(&payload, InsertConflict::Write(&payload))?);
            }
            self.execute_statements(Column::ATTRIBUTE, &statements)
                .await
        })
        .await
    }
}

// Prisma's model_dump(exclude_none=True) leaves unset columns out of the write
fn without_nulls<Column>(payload: &BTreeMap<Column, TokenValue>) -> BTreeMap<Column, TokenValue>
where
    Column: TableColumn,
{
    payload
        .iter()
        .filter(|(_, value)| !matches!(value, TokenValue::Null))
        .map(|(column, value)| (*column, value.clone()))
        .collect()
}

fn token_key<ClientError>(
    payload: &BTreeMap<TokenColumn, TokenValue>,
) -> Result<String, TableOpsError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
{
    let (key_name, _) = TokenColumn::KEY.sql_definition();
    match payload.get(&TokenColumn::Token) {
        Some(TokenValue::Text(text)) => Ok(text.clone()),
        Some(TokenValue::KeyHash(hash)) => Ok(String::from(hash.expose_secret())),
        Some(_) => Err(TableOpsError::WrongType { column: key_name }),
        None => Err(TableOpsError::MissingField { field: key_name }),
    }
}

fn hash_key_row(payload: &BTreeMap<TokenColumn, TokenValue>) -> BTreeMap<TokenColumn, TokenValue> {
    let mut row = without_nulls(payload);
    if let Some(TokenValue::Text(token)) = row.get(&TokenColumn::Token) {
        let hashed = hash_token_if_needed(token);
        row.insert(TokenColumn::Token, TokenValue::Text(hashed));
    }
    row
}
