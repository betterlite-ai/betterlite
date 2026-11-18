use std::collections::BTreeMap;

use crate::database_replica::client::DatabaseClient;
use crate::key_repository::table::TokenValue;
use crate::key_repository::table::columns::TokenColumn;
use crate::key_user_writes::columns::UserColumn;
use crate::key_user_writes::hash::hash_token_if_needed;
use crate::postgres_store::diagnostics::DriverError;

use crate::table_ops::access::{
    BUDGET_ATTRIBUTE, END_USER_ATTRIBUTE, KEY_ATTRIBUTE, TEAM_ATTRIBUTE, TablePoolAccess,
    USER_ATTRIBUTE,
};
use crate::table_ops::columns::{BudgetColumn, EndUserColumn, TableColumn, TeamColumn};
use crate::table_ops::sql::{self, Statement};
use crate::table_ops::team_json::{jsonify_object, jsonify_team_object};
use crate::table_ops::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

impl<Client> TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    pub async fn update_many_keys(
        &self,
        rows: &[BTreeMap<TokenColumn, TokenValue>],
    ) -> Result<(), TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::UpdateData, || {
            self.update_many_keys_once(rows)
        })
        .await
    }

    pub async fn update_many_users(
        &self,
        rows: &[BTreeMap<UserColumn, TokenValue>],
    ) -> Result<(), TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::UpdateData, || {
            self.update_many_users_once(rows)
        })
        .await
    }

    pub async fn update_many_end_users(
        &self,
        rows: &[BTreeMap<EndUserColumn, TokenValue>],
    ) -> Result<(), TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::UpdateData, || {
            self.update_many_end_users_once(rows)
        })
        .await
    }

    pub async fn update_many_budgets(
        &self,
        rows: &[BTreeMap<BudgetColumn, TokenValue>],
    ) -> Result<(), TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::UpdateData, || {
            self.update_many_budgets_once(rows)
        })
        .await
    }

    pub async fn update_many_teams(
        &self,
        rows: &[BTreeMap<TeamColumn, TokenValue>],
    ) -> Result<(), TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::UpdateData, || {
            self.update_many_teams_once(rows)
        })
        .await
    }

    async fn update_many_keys_once(
        &self,
        rows: &[BTreeMap<TokenColumn, TokenValue>],
    ) -> Result<(), TableOpsError<Client::Error>> {
        let mut statements = Vec::new();
        for row in rows {
            statements.push(sql::update_by_key(&hash_key_row(row))?);
        }
        self.execute_batch(KEY_ATTRIBUTE, statements).await
    }

    async fn update_many_users_once(
        &self,
        rows: &[BTreeMap<UserColumn, TokenValue>],
    ) -> Result<(), TableOpsError<Client::Error>> {
        let mut statements = Vec::new();
        for row in rows {
            let payload = jsonify_object(&without_nulls(row));
            statements.push(sql::upsert(&payload, Some(&payload))?);
        }
        self.execute_batch(USER_ATTRIBUTE, statements).await
    }

    async fn update_many_end_users_once(
        &self,
        rows: &[BTreeMap<EndUserColumn, TokenValue>],
    ) -> Result<(), TableOpsError<Client::Error>> {
        let mut statements = Vec::new();
        for row in rows {
            let payload = jsonify_object(&without_nulls(row));
            statements.push(sql::upsert(&payload, Some(&payload))?);
        }
        self.execute_batch(END_USER_ATTRIBUTE, statements).await
    }

    async fn update_many_budgets_once(
        &self,
        rows: &[BTreeMap<BudgetColumn, TokenValue>],
    ) -> Result<(), TableOpsError<Client::Error>> {
        let mut statements = Vec::new();
        for row in rows {
            let payload = jsonify_object(&without_nulls(row));
            statements.push(sql::upsert(&payload, Some(&payload))?);
        }
        self.execute_batch(BUDGET_ATTRIBUTE, statements).await
    }

    async fn update_many_teams_once(
        &self,
        rows: &[BTreeMap<TeamColumn, TokenValue>],
    ) -> Result<(), TableOpsError<Client::Error>> {
        let mut statements = Vec::new();
        for row in rows {
            let payload = jsonify_team_object(&without_nulls(row));
            statements.push(sql::upsert(&payload, Some(&payload))?);
        }
        self.execute_batch(TEAM_ATTRIBUTE, statements).await
    }

    // LiteLLM writes the whole batch through one transaction, so a failure loses every row in it
    async fn execute_batch(
        &self,
        attribute: &str,
        statements: Vec<Statement>,
    ) -> Result<(), TableOpsError<Client::Error>> {
        let mut client = self.connection(attribute, "update_many").await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|driver| TableOpsError::from(DriverError { driver }))?;
        for statement in &statements {
            transaction
                .execute(&statement.sql, &statement.parameters.bind_values())
                .await
                .map_err(|driver| TableOpsError::from(DriverError { driver }))?;
        }
        transaction
            .commit()
            .await
            .map_err(|driver| TableOpsError::from(DriverError { driver }))?;
        Ok(())
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

fn hash_key_row(payload: &BTreeMap<TokenColumn, TokenValue>) -> BTreeMap<TokenColumn, TokenValue> {
    let mut row = without_nulls(payload);
    if let Some(TokenValue::Text(token)) = row.get(&TokenColumn::Token) {
        let hashed = hash_token_if_needed(token);
        row.insert(TokenColumn::Token, TokenValue::Text(hashed));
    }
    row
}
