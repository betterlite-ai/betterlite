use std::convert::Infallible;

use async_trait::async_trait;
use axum::http::StatusCode;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::{DateTime, NaiveDateTime, Utc};
use thiserror::Error;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use crate::user_invitation::{InvitationError, InvitationRecord, create_invitation_for_user};

// create_invitation_for_user's proxy_admin_name, which the route does not take from a config
pub const PROXY_ADMIN_NAME: &str = "default_user_id";

const SELECT_SQL: &str = concat!(
    r#"SELECT "#,
    r#""id", "user_id", "is_accepted", "accepted_at", "expires_at", "created_at", "created_by", "updated_at", "updated_by""#,
    r#" FROM "LiteLLM_InvitationLink" WHERE "id" = $1"#
);

#[derive(Debug, Error)]
pub enum InvitationStoreError {
    #[error(transparent)]
    Create(#[from] InvitationError),
    #[error("the invitation row could not be read")]
    Read(#[source] DbError),
    #[error("the invitation row could not be updated")]
    Update(#[source] DbError),
    #[error("the invitation row could not be deleted")]
    Delete(#[source] DbError),
}

impl InvitationStoreError {
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::Create(error) => error.status(),
            Self::Read(_) | Self::Update(_) | Self::Delete(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Create(error) => error.message(),
            Self::Read(error) | Self::Update(error) | Self::Delete(error) => error.to_string(),
        }
    }
}

// The InvitationLinkRepository calls the four invitation routes make
#[async_trait]
pub trait InvitationStore: Send + Sync {
    async fn create(
        &self,
        user_id: &str,
        actor_user_id: Option<&str>,
    ) -> Result<InvitationRecord, InvitationStoreError>;

    async fn info(
        &self,
        invitation_id: &str,
    ) -> Result<Option<InvitationRecord>, InvitationStoreError>;

    async fn update(
        &self,
        invitation_id: &str,
        is_accepted: bool,
        updated_by: &str,
    ) -> Result<Option<InvitationRecord>, InvitationStoreError>;

    async fn delete(
        &self,
        invitation_id: &str,
    ) -> Result<Option<InvitationRecord>, InvitationStoreError>;
}

pub struct PostgresInvitations {
    pub store: PostgresStore,
}

#[async_trait]
impl InvitationStore for PostgresInvitations {
    async fn create(
        &self,
        user_id: &str,
        actor_user_id: Option<&str>,
    ) -> Result<InvitationRecord, InvitationStoreError> {
        create_invitation_for_user(Some(&self.store), user_id, actor_user_id, PROXY_ADMIN_NAME)
            .await
            .map_err(InvitationStoreError::Create)
    }

    async fn info(
        &self,
        invitation_id: &str,
    ) -> Result<Option<InvitationRecord>, InvitationStoreError> {
        let client = self
            .store
            .session()
            .await
            .map_err(InvitationStoreError::Read)?;
        let row = client
            .query_opt(SELECT_SQL, &[&invitation_id])
            .await
            .map_err(|driver| InvitationStoreError::Read(backend::<Infallible>(driver)))?;
        row.as_ref()
            .map(decode_row)
            .transpose()
            .map_err(InvitationStoreError::Read)
    }

    async fn update(
        &self,
        invitation_id: &str,
        is_accepted: bool,
        updated_by: &str,
    ) -> Result<Option<InvitationRecord>, InvitationStoreError> {
        let client = self
            .store
            .session()
            .await
            .map_err(InvitationStoreError::Update)?;
        let now = Utc::now().naive_utc();
        let parameters: [&(dyn ToSql + Sync); 4] =
            [&invitation_id, &is_accepted, &now, &updated_by];
        let statement = concat!(
            r#"UPDATE "LiteLLM_InvitationLink" SET "is_accepted" = $2, "accepted_at" = $3, "updated_at" = $3, "updated_by" = $4 WHERE "id" = $1 RETURNING "#,
            r#""id", "user_id", "is_accepted", "accepted_at", "expires_at", "created_at", "created_by", "updated_at", "updated_by""#
        );
        let row = client
            .query_opt(statement, &parameters)
            .await
            .map_err(|driver| InvitationStoreError::Update(backend::<Infallible>(driver)))?;
        row.as_ref()
            .map(decode_row)
            .transpose()
            .map_err(InvitationStoreError::Update)
    }

    async fn delete(
        &self,
        invitation_id: &str,
    ) -> Result<Option<InvitationRecord>, InvitationStoreError> {
        let client = self
            .store
            .session()
            .await
            .map_err(InvitationStoreError::Delete)?;
        let statement = concat!(
            r#"DELETE FROM "LiteLLM_InvitationLink" WHERE "id" = $1 RETURNING "#,
            r#""id", "user_id", "is_accepted", "accepted_at", "expires_at", "created_at", "created_by", "updated_at", "updated_by""#
        );
        let row = client
            .query_opt(statement, &[&invitation_id])
            .await
            .map_err(|driver| InvitationStoreError::Delete(backend::<Infallible>(driver)))?;
        row.as_ref()
            .map(decode_row)
            .transpose()
            .map_err(InvitationStoreError::Delete)
    }
}

// B10092 decodes the same columns for its insert, and its decoder is private to that module
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
        accepted_at: accepted_at.map(|value| value.and_utc()),
        expires_at: expires_at.and_utc(),
        created_at: created_at.and_utc(),
        created_by: row.try_get("created_by").map_err(backend::<Infallible>)?,
        updated_at: updated_at.and_utc(),
        updated_by: row.try_get("updated_by").map_err(backend::<Infallible>)?,
    })
}
