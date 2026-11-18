use deadpool_postgres::GenericClient;
use secrecy::{ExposeSecret as _, SecretString};

use crate::budget_permission_rows::Timestamp;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::key_repository::decode::to_model;
use crate::key_repository::record::DbRecord;
use crate::key_repository::table::columns::TokenColumn;
use crate::key_repository::table::{TokenValue, TokenWrite};
use crate::postgres_store::errors::DbError;

use super::columns::{UserColumn, UserColumnOp, UserUpdateOps, UserWrite};
use super::hash::hash_token_if_needed;
use super::retry::CallType;
use super::rows;
use super::sql::{self, Relation, Statement};
use super::{
    KeyUpdateResult, KeyUserWriteError, KeyUserWrites, UserUpdate, UserUpdateResult, WriteRowError,
};

impl<Client> KeyUserWrites<'_, Client>
where
    Client: GenericClient + Sync,
{
    pub async fn update_key(
        &self,
        token: &SecretString,
        fields: &TokenWrite,
    ) -> Result<KeyUpdateResult, KeyUserWriteError> {
        self.run(CallType::UpdateData, || {
            self.attempt_update_key(token, fields)
        })
        .await
    }

    pub(super) async fn attempt_update_key(
        &self,
        token: &SecretString,
        fields: &TokenWrite,
    ) -> Result<KeyUpdateResult, KeyUserWriteError> {
        let lookup = hash_token_if_needed(token.expose_secret());
        let now = Timestamp::Aware(self.ports.clock.now().fixed_offset());
        let mut data = fields.clone();
        data.0
            .insert(TokenColumn::Token, TokenValue::Text(lookup.clone()));
        data.0
            .insert(TokenColumn::SettingsUpdatedAt, TokenValue::Timestamp(now));
        data.0
            .entry(TokenColumn::UpdatedAt)
            .or_insert_with(|| TokenValue::Timestamp(now));
        let mut statement = Statement::default();
        let query = sql::update_key(&mut statement, &data)?;
        let found = self.query_row(&query, &statement).await?;
        tracing::debug!("DB Token Table update succeeded");
        let Some(row) = found else {
            return Ok(KeyUpdateResult {
                token: SecretString::from(lookup),
                data: None,
            });
        };
        let mut record = rows::row_object(&row)?;
        rows::fill_null_lists(&mut record, &rows::KEY_LIST_COLUMNS);
        sql::attach_relation(self.client, &mut record, Relation::ObjectPermission).await?;
        let stored = DbRecord {
            model_dump: None,
            dict: Some(record),
            mapping: None,
            pairs: None,
        };
        let decoded = to_model(Some(&stored)).map_err(WriteRowError::Key)?;
        Ok(KeyUpdateResult {
            token: SecretString::from(lookup),
            data: decoded,
        })
    }

    pub(super) async fn attempt_update_user(
        &self,
        request: &UserUpdate,
    ) -> Result<UserUpdateResult, KeyUserWriteError> {
        let from_data = request.fields.text(UserColumn::UserId);
        let user_id = request
            .user_id
            .as_deref()
            .or(from_data)
            .ok_or(KeyUserWriteError::MissingField { field: "user_id" })?
            .to_owned();
        let create = request.fields.clone();
        let mut update = match (&request.update_fields, &request.custom_update_fields) {
            (Some(fields), _) => UserUpdateOps::from(fields),
            (None, Some(ops)) => ops.clone(),
            (None, None) => UserUpdateOps::from(&create),
        };
        if !update.0.is_empty() {
            let now = Timestamp::Aware(self.ports.clock.now().fixed_offset());
            update
                .0
                .entry(UserColumn::UpdatedAt)
                .or_insert(UserColumnOp::Set(TokenValue::Timestamp(now)));
        }
        let data = if from_data == Some(user_id.as_str()) {
            self.upsert_user_row(&create, &update).await?
        } else {
            self.update_or_insert_user_row(&user_id, &create, &update)
                .await?
        };
        tracing::info!("DB User Table - update succeeded");
        Ok(UserUpdateResult { user_id, data })
    }

    async fn update_or_insert_user_row(
        &self,
        lookup: &str,
        create: &UserWrite,
        update: &UserUpdateOps,
    ) -> Result<LiteLLMUserTable, KeyUserWriteError> {
        let mut statement = Statement::default();
        let query = sql::update_user(&mut statement, lookup, update)?;
        if let Some(row) = self.query_row(&query, &statement).await? {
            return rows::decode_user_row(&row);
        }
        let mut statement = Statement::default();
        let query = sql::insert_user(&mut statement, create, sql::UserConflict::Reject)?;
        let Some(row) = self.query_row(&query, &statement).await? else {
            return Err(KeyUserWriteError::Database {
                source: DbError::CreateNull,
            });
        };
        rows::decode_user_row(&row)
    }
}
