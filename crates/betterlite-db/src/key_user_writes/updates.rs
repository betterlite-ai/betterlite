use deadpool_postgres::GenericClient;
use secrecy::{ExposeSecret as _, SecretString};

use crate::budget_permission_rows::Timestamp;
use crate::key_repository::decode::to_model;
use crate::key_repository::record::DbRecord;
use crate::key_repository::table::columns::TokenColumn;
use crate::key_repository::table::{TokenValue, TokenWrite};

use super::columns::{UserColumn, UserWrite};
use super::hash::hash_token_if_needed;
use super::rows;
use super::sql::{self, Relation, Statement};
use super::{
    KeyUpdateResult, KeyUserWriteError, KeyUserWrites, UserUpdate, UserUpdateResult, WriteRowError,
};

impl<Client> KeyUserWrites<'_, Client>
where
    Client: GenericClient + Sync,
{
    pub(super) async fn attempt_update_key(
        &self,
        token: &SecretString,
        fields: &TokenWrite,
    ) -> Result<KeyUpdateResult, KeyUserWriteError> {
        let lookup = hash_token_if_needed(token.expose_secret());
        let mut data = fields.clone();
        data.0
            .insert(TokenColumn::Token, TokenValue::Text(lookup.clone()));
        data.0.insert(
            TokenColumn::SettingsUpdatedAt,
            TokenValue::Timestamp(Timestamp::Aware(self.ports.clock.now().fixed_offset())),
        );
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
        let mut create = request.fields.clone();
        create
            .0
            .insert(UserColumn::UserId, TokenValue::Text(user_id.clone()));
        let chosen = request
            .update_fields
            .as_ref()
            .or(request.custom_update_fields.as_ref());
        let update = match chosen {
            Some(fields) => fields.clone(),
            None => create.clone(),
        };
        let data = self.upsert_user_row(&create, &update).await?;
        tracing::info!("DB User Table - update succeeded");
        Ok(UserUpdateResult { user_id, data })
    }
}
