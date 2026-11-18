use std::collections::BTreeMap;

use deadpool_postgres::GenericClient;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use crate::budget_permission_rows::Timestamp;
use crate::identity_rows::key::LiteLLMVerificationToken;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::key_repository::decode::{decode_mapping, to_model};
use crate::key_repository::record::DbRecord;
use crate::key_repository::table::columns::TokenColumn;
use crate::key_repository::table::{TokenValue, TokenWrite};
use crate::postgres_store::errors::{DbError, backend};

use super::columns::{UserColumn, UserUpdateOps, UserWrite};
use super::hash::{hash_token, hash_token_if_needed};
use super::retry::{CallType, run};
use super::rows;
use super::sql::{self, Relation, Statement};
use super::translate_organization_error;
use super::updates::update_operations;
use super::{
    KeyUpdateResult, KeyUserWriteError, KeyUserWrites, UserUpdate, UserUpdateResult, WriteRowError,
};

const REDACTED: &str = "[redacted]";

type InsertedKey = Result<LiteLLMVerificationToken, KeyUserWriteError>;
type InsertedUser = Result<LiteLLMUserTable, KeyUserWriteError>;
type UpdatedKey = Result<KeyUpdateResult, KeyUserWriteError>;
type UpdatedUser = Result<UserUpdateResult, KeyUserWriteError>;

impl<Client> KeyUserWrites<'_, Client>
where
    Client: GenericClient + Sync,
{
    pub async fn insert_key(&self, token: &SecretString, fields: &TokenWrite) -> InsertedKey {
        let hashed = hash_token(token.expose_secret());
        run(&self.ports, CallType::InsertData, || {
            self.attempt_insert_key(&hashed, fields)
        })
        .await
    }

    pub async fn insert_user(&self, fields: &UserWrite) -> InsertedUser {
        run(&self.ports, CallType::InsertData, || {
            self.attempt_insert_user(fields)
        })
        .await
    }

    pub async fn update_user(&self, request: &UserUpdate) -> UpdatedUser {
        run(&self.ports, CallType::UpdateData, || {
            self.attempt_update_user(request)
        })
        .await
    }

    pub async fn update_key(&self, token: &SecretString, fields: &TokenWrite) -> UpdatedKey {
        run(&self.ports, CallType::UpdateData, || {
            self.attempt_update_key(token, fields)
        })
        .await
    }

    async fn attempt_insert_key(
        &self,
        hashed: &str,
        fields: &TokenWrite,
    ) -> Result<LiteLLMVerificationToken, KeyUserWriteError> {
        tracing::debug!(payload = ?debug_payload(fields, hashed), "PrismaClient: insert_data");
        let mut data = fields.clone();
        data.0
            .insert(TokenColumn::Token, TokenValue::Text(hashed.to_owned()));
        if matches!(
            data.0.get(&TokenColumn::BudgetLimits),
            Some(TokenValue::Null)
        ) {
            data.0.remove(&TokenColumn::BudgetLimits);
        }
        let mut statement = Statement::default();
        let query = sql::upsert_key(&mut statement, &data)?;
        tracing::debug!("PrismaClient: Before upsert into litellm_verificationtoken");
        let Some(row) = self.query_row(&query, &statement).await? else {
            return Err(KeyUserWriteError::Database {
                source: DbError::CreateNull,
            });
        };
        let mut record = rows::row_object(&row)?;
        rows::fill_null_lists(&mut record, &rows::KEY_LIST_COLUMNS);
        sql::attach_relation(self.client, &mut record, Relation::Budget).await?;
        let decoded = decode_mapping(record).map_err(WriteRowError::Key)?;
        tracing::info!("Data Inserted into Keys Table");
        Ok(decoded)
    }

    async fn attempt_insert_user(
        &self,
        fields: &UserWrite,
    ) -> Result<LiteLLMUserTable, KeyUserWriteError> {
        if fields.text(UserColumn::UserId).is_none() {
            return Err(KeyUserWriteError::MissingField { field: "user_id" });
        }
        let payload: BTreeMap<&'static str, String> = fields
            .0
            .iter()
            .map(|(column, value)| {
                let text = if *column == UserColumn::Password {
                    String::from(REDACTED)
                } else {
                    format!("{value:?}")
                };
                (column.definition().name, text)
            })
            .collect();
        tracing::debug!(payload = ?payload, "PrismaClient: insert_data");
        match self
            .upsert_user_row(fields, &UserUpdateOps::default())
            .await
        {
            Ok(user) => {
                tracing::info!("Data Inserted into User Table");
                Ok(user)
            }
            Err(error) => Err(translate_organization_error(error, fields)),
        }
    }

    pub(super) async fn upsert_user_row(
        &self,
        fields: &UserWrite,
        update: &UserUpdateOps,
    ) -> Result<LiteLLMUserTable, KeyUserWriteError> {
        let mut statement = Statement::default();
        let query = sql::insert_user(&mut statement, fields, sql::UserConflict::Upsert(update))?;
        let Some(row) = self.query_row(&query, &statement).await? else {
            return Err(KeyUserWriteError::Database {
                source: DbError::CreateNull,
            });
        };
        rows::decode_user_row(&row)
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
        let now = Timestamp::Aware(self.ports.clock.now().fixed_offset());
        let update = update_operations(request, &create, now);
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
        let updated = {
            let mut statement = Statement::default();
            let query = sql::update_user(&mut statement, lookup, update)?;
            self.query_row(&query, &statement).await?
        };
        if let Some(row) = updated {
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

    pub(super) async fn query_row(
        &self,
        sql: &str,
        statement: &Statement,
    ) -> Result<Option<Row>, KeyUserWriteError> {
        let parameters: Vec<&(dyn ToSql + Sync)> = statement
            .parameters
            .iter()
            .map(sql::Parameter::bind)
            .collect();
        self.client
            .query_opt(sql, &parameters)
            .await
            .map_err(|driver| KeyUserWriteError::Database {
                source: backend::<WriteRowError>(driver),
            })
    }
}

// the source logs the whole insert payload, with the raw key replaced by the hash it stores
fn debug_payload(fields: &TokenWrite, hashed: &str) -> BTreeMap<&'static str, String> {
    let mut payload = BTreeMap::new();
    for (column, value) in &fields.0 {
        let text = if *column == TokenColumn::Token {
            hashed.to_owned()
        } else {
            format!("{value:?}")
        };
        payload.insert(column.name(), text);
    }
    payload
}
