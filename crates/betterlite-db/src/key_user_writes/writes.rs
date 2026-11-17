use std::collections::BTreeMap;
use std::future::Future;

use deadpool_postgres::GenericClient;
use secrecy::SecretString;
use tokio_postgres::Row;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::ToSql;

use crate::identity_rows::key::LiteLLMVerificationToken;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::key_repository::decode::decode_mapping;
use crate::key_repository::table::columns::TokenColumn;
use crate::key_repository::table::{TokenValue, TokenWrite};
use crate::postgres_store::errors::{DbError, backend};
use crate::user_repository::projection::decode_user_record;

use super::columns::{UserColumn, UserWrite};
use super::hash::hash_token;
use super::retry::{CallType, FailureEvent, MAX_TIME, MAX_TRIES, retry_wait, traceback};
use super::rows;
use super::sql::{self, Relation, Statement};
use super::{
    KeyUpdateResult, KeyUserWriteError, KeyUserWrites, UserUpdate, UserUpdateResult, WriteRowError,
};

const ORGANIZATION_FOREIGN_KEY: &str = "LiteLLM_UserTable_organization_id_fkey";

impl<Client> KeyUserWrites<'_, Client>
where
    Client: GenericClient + Sync,
{
    pub async fn insert_key(
        &self,
        fields: &TokenWrite,
    ) -> Result<LiteLLMVerificationToken, KeyUserWriteError> {
        self.run(CallType::InsertData, || self.attempt_insert_key(fields))
            .await
    }

    pub async fn insert_user(
        &self,
        fields: &UserWrite,
    ) -> Result<LiteLLMUserTable, KeyUserWriteError> {
        self.run(CallType::InsertData, || self.attempt_insert_user(fields))
            .await
    }

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

    pub async fn update_user(
        &self,
        request: &UserUpdate,
    ) -> Result<UserUpdateResult, KeyUserWriteError> {
        self.run(CallType::UpdateData, || self.attempt_update_user(request))
            .await
    }

    async fn run<Output, Attempt, AttemptFuture>(
        &self,
        call_type: CallType,
        attempt: Attempt,
    ) -> Result<Output, KeyUserWriteError>
    where
        Output: Send,
        Attempt: Fn() -> AttemptFuture + Send,
        AttemptFuture: Future<Output = Result<Output, KeyUserWriteError>> + Send,
    {
        let start = self.ports.clock.instant();
        let mut tries: u32 = 0;
        loop {
            tries = tries.saturating_add(1);
            let elapsed = self.ports.clock.instant().saturating_duration_since(start);
            let attempt_start = self.ports.clock.instant();
            let outcome = attempt().await;
            let duration = self
                .ports
                .clock
                .instant()
                .saturating_duration_since(attempt_start);
            let error = match outcome {
                Ok(output) => return Ok(output),
                Err(error) => error,
            };
            let rendered = traceback(call_type, &error);
            let error_msg = rendered.lines().next().unwrap_or_default();
            tracing::debug!("{error_msg}");
            self.ports
                .sink
                .record(FailureEvent {
                    call_type,
                    error: error.to_string(),
                    traceback: rendered,
                    duration,
                })
                .await;
            if tries == MAX_TRIES || elapsed >= MAX_TIME {
                return Err(error);
            }
            let wait = retry_wait(self.ports.jitter, tries, elapsed);
            tracing::debug!("Backing off... this was attempt #{tries}");
            self.ports.pause.pause(wait).await;
        }
    }

    async fn attempt_insert_key(
        &self,
        fields: &TokenWrite,
    ) -> Result<LiteLLMVerificationToken, KeyUserWriteError> {
        let Some(TokenValue::Text(token)) = fields.0.get(&TokenColumn::Token) else {
            return Err(KeyUserWriteError::MissingField { field: "token" });
        };
        let hashed = hash_token(token);
        tracing::debug!(payload = ?debug_payload(fields, &hashed), "PrismaClient: insert_data");
        let mut data = fields.clone();
        data.0.insert(TokenColumn::Token, TokenValue::Text(hashed));
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
        tracing::debug!(payload = ?fields, "PrismaClient: insert_data");
        match self.upsert_user_row(fields, &UserWrite::default()).await {
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
        update: &UserWrite,
    ) -> Result<LiteLLMUserTable, KeyUserWriteError> {
        let mut statement = Statement::default();
        let query = sql::upsert_user(&mut statement, fields, update)?;
        let Some(row) = self.query_row(&query, &statement).await? else {
            return Err(KeyUserWriteError::Database {
                source: DbError::CreateNull,
            });
        };
        let text = rows::row_text(&row)?;
        let decoded = decode_user_record(text.as_bytes()).map_err(WriteRowError::User)?;
        decoded.ok_or(KeyUserWriteError::Database {
            source: DbError::CreateNull,
        })
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

fn translate_organization_error(error: KeyUserWriteError, fields: &UserWrite) -> KeyUserWriteError {
    if !is_named_organization_failure(&error) {
        return error;
    }
    match fields.text(UserColumn::OrganizationId) {
        Some(organization_id) => KeyUserWriteError::OrganizationMissing {
            organization_id: organization_id.to_owned(),
        },
        None => error,
    }
}

fn is_named_organization_failure(error: &KeyUserWriteError) -> bool {
    match error {
        KeyUserWriteError::Database {
            source: DbError::SqlState {
                code, constraint, ..
            },
        } => {
            code == &SqlState::FOREIGN_KEY_VIOLATION
                && constraint.as_deref() == Some(ORGANIZATION_FOREIGN_KEY)
        }
        KeyUserWriteError::Database { .. }
        | KeyUserWriteError::Row(_)
        | KeyUserWriteError::Encode { .. }
        | KeyUserWriteError::MissingField { .. }
        | KeyUserWriteError::OrganizationMissing { .. } => false,
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
