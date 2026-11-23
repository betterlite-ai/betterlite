use std::convert::Infallible;
use std::error::Error as _;

use async_trait::async_trait;
use axum::http::StatusCode;
use betterlite_db::database_faults::native::native_sqlstate;
use betterlite_db::postgres_store::diagnostics::DriverError;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use betterlite_logging::key_invitation_email::link::{InvitationCreator, InvitationId};
use chrono::{DateTime, NaiveDateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_postgres::Row;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::ToSql;
use uuid::Uuid;

const USER_MISSING: &str =
    "User id does not exist in 'LiteLLM_UserTable'. Fix this by creating user via `/user/new`.";

const INSERT_SQL: &str = r#"INSERT INTO "LiteLLM_InvitationLink" ("id", "user_id", "created_at", "expires_at", "created_by", "updated_at", "updated_by") VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING "id", "user_id", "is_accepted", "accepted_at", "expires_at", "created_at", "created_by", "updated_at", "updated_by""#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvitationRecord {
    pub id: String,
    pub user_id: String,
    pub is_accepted: bool,
    pub accepted_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub updated_at: DateTime<Utc>,
    pub updated_by: String,
}

#[derive(Debug, Error)]
pub enum InvitationError {
    #[error("the database is not connected")]
    NoDatabase,
    #[error("the invited user does not exist")]
    UserMissing,
    #[error("the invitation row could not be inserted")]
    Insert(#[source] DbError),
}

impl InvitationError {
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::NoDatabase | Self::UserMissing => StatusCode::BAD_REQUEST,
            Self::Insert(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::NoDatabase => CommonProxyErrors::DbNotConnectedError.as_str().to_owned(),
            Self::UserMissing => USER_MISSING.to_owned(),
            Self::Insert(error) => database_text(error),
        }
    }
}

// LiteLLM answers a failed insert with the database's own text, and the driver holds the server's message
fn database_text(error: &DbError) -> String {
    let Some(diagnostics) = error
        .source()
        .and_then(|source| source.downcast_ref::<DriverError>())
    else {
        return error.to_string();
    };
    match diagnostics.driver.as_db_error() {
        Some(database) => database.message().to_owned(),
        None => error.to_string(),
    }
}

pub async fn create_invitation_for_user(
    store: Option<&PostgresStore>,
    user_id: &str,
    actor_user_id: Option<&str>,
    proxy_admin_name: &str,
) -> Result<InvitationRecord, InvitationError> {
    let Some(database) = store else {
        return Err(InvitationError::NoDatabase);
    };
    let now = Utc::now().naive_utc();
    // an expiry past the representable range saturates, as the token expiry helpers do
    let expires_at = now
        .checked_add_signed(TimeDelta::days(7))
        .unwrap_or(NaiveDateTime::MAX);
    let actor = actor_user_id
        .filter(|caller| !caller.is_empty())
        .unwrap_or(proxy_admin_name);
    let id = Uuid::new_v4().to_string();
    let client = database.session().await.map_err(InvitationError::Insert)?;
    let parameters: [&(dyn ToSql + Sync); 7] =
        [&id, &user_id, &now, &expires_at, &actor, &now, &actor];
    let row = match client.query_one(INSERT_SQL, &parameters).await {
        Ok(row) => row,
        Err(source) => {
            let error = backend::<Infallible>(source);
            // LiteLLM reads Prisma's foreign key text; the driver reports SQLSTATE 23503 instead
            return Err(
                if native_sqlstate(&error) == Some(SqlState::FOREIGN_KEY_VIOLATION) {
                    InvitationError::UserMissing
                } else {
                    InvitationError::Insert(error)
                },
            );
        }
    };
    decode_row(&row).map_err(InvitationError::Insert)
}

fn decode_row(row: &Row) -> Result<InvitationRecord, DbError> {
    let accepted_at: Option<NaiveDateTime> =
        row.try_get("accepted_at").map_err(backend::<Infallible>)?;
    let expires_at: NaiveDateTime = row.try_get("expires_at").map_err(backend::<Infallible>)?;
    let created_at: NaiveDateTime = row.try_get("created_at").map_err(backend::<Infallible>)?;
    let updated_at: NaiveDateTime = row.try_get("updated_at").map_err(backend::<Infallible>)?;
    Ok(InvitationRecord {
        id: row.try_get("id").map_err(backend::<Infallible>)?,
        user_id: row.try_get("user_id").map_err(backend::<Infallible>)?,
        is_accepted: row.try_get("is_accepted").map_err(backend::<Infallible>)?,
        accepted_at: accepted_at.map(NaiveDateTime::and_utc),
        expires_at: expires_at.and_utc(),
        created_at: created_at.and_utc(),
        created_by: row.try_get("created_by").map_err(backend::<Infallible>)?,
        updated_at: updated_at.and_utc(),
        updated_by: row.try_get("updated_by").map_err(backend::<Infallible>)?,
    })
}

pub struct InvitationLinks {
    pub store: PostgresStore,
    pub proxy_admin_name: String,
}

#[async_trait]
impl InvitationCreator for InvitationLinks {
    type Error = InvitationError;

    async fn create_for_user(&self, user_id: &str) -> Result<InvitationId, InvitationError> {
        let record = create_invitation_for_user(
            Some(&self.store),
            user_id,
            Some(user_id),
            &self.proxy_admin_name,
        )
        .await?;
        Ok(InvitationId(record.id))
    }
}

mod tests;
