use std::sync::Arc;

use async_trait::async_trait;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_auth::auth_read::include::UserInclude;
use betterlite_auth::auth_read::repository::{NativeKey, RepositoryTable};
use betterlite_auth::auth_read::table::{AuthReadError, AuthTable as _, FindOne, Presence};
use betterlite_auth::auth_user_lookup::rows::{UserRowError, UserRows};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::rows::KeyColumn;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::user_repository::projection::{UserRecordError, user_record_to_model};
use betterlite_db::user_repository::{RowDecodeError, UserRepository};
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use serde::Serialize;

use crate::organization_create::caller::OrgCaller;

pub const NO_ACCESS_DETAIL: &str = "You do not have access to this organization";
pub const ORG_ADMIN_ROLE: &str = "org_admin";

#[derive(Debug, thiserror::Error)]
pub enum OrgAccessError {
    #[error("You do not have access to this organization")]
    Denied,
    #[error("the database is not connected")]
    DatabaseNotConnected,
    #[error("the caller user session failed: {0}")]
    Session(#[source] DbError),
    #[error("the caller user row could not be read: {0}")]
    UserRead(#[source] DbError<RowDecodeError>),
    #[error("the caller membership rows could not be read: {0}")]
    MembershipRead(#[source] AuthReadError<UserRowError>),
    #[error("the stored user record is not readable: {0}")]
    Record(#[source] UserRecordError),
}

// B736's get_user_object(user_id_upsert=false), with the row the membership check reads
#[async_trait]
pub trait OrgUserLookup: Send + Sync {
    async fn find_user(&self, user_id: &str) -> Result<Option<LiteLLMUserTable>, OrgAccessError>;
}

pub struct DatabaseUserLookup<'store> {
    pub store: &'store PostgresStore,
}

#[async_trait]
impl OrgUserLookup for DatabaseUserLookup<'_> {
    async fn find_user(&self, user_id: &str) -> Result<Option<LiteLLMUserTable>, OrgAccessError> {
        let session = self
            .store
            .session()
            .await
            .map_err(OrgAccessError::Session)?;
        // B736's user rows and include, read without the cache and deadline the auth stack supplies
        let table = RepositoryTable {
            client: Some(Arc::new(session)),
            table: UserRows,
        };
        let found = table
            .find_unique(FindOne {
                filter: NativeKey {
                    column: KeyColumn::Id,
                    value: Box::new(user_id.to_owned()),
                },
                include: Presence::Value(UserInclude {
                    organization_memberships: true,
                }),
            })
            .await
            .map_err(OrgAccessError::MembershipRead)?;
        user_record_to_model(found.as_ref()).map_err(OrgAccessError::Record)
    }
}

// LiteLLM's _verify_org_access: an admin view passes, everyone else needs an org_admin membership of this organization
pub async fn verify_org_access(
    organization_id: &str,
    caller: &OrgCaller,
    lookup: &dyn OrgUserLookup,
) -> Result<(), OrgAccessError> {
    if caller.admin_view {
        return Ok(());
    }
    let Some(user_id) = caller.user_id.as_deref() else {
        return Err(OrgAccessError::Denied);
    };
    let Some(user) = lookup.find_user(user_id).await? else {
        return Err(OrgAccessError::Denied);
    };
    let memberships = user.organization_memberships.as_deref().unwrap_or_default();
    let is_admin = memberships.iter().any(|membership| {
        membership.organization_id == organization_id
            && membership.user_role.as_deref() == Some(ORG_ADMIN_ROLE)
    });
    if is_admin {
        return Ok(());
    }
    Err(OrgAccessError::Denied)
}

// the caller's stored user row; creation swallows every lookup and validation failure, as the source does
pub async fn read_user_object(
    store: &PostgresStore,
    caller: &OrgCaller,
) -> Option<LiteLLMUserTable> {
    let user_id = caller.user_id.as_deref()?;
    match read_creation_user(store, user_id).await {
        Ok(found) => found,
        Err(error) => {
            tracing::debug!(error = %error, "the caller user row could not be read");
            None
        }
    }
}

// _table(UserRepository(prisma_client)).find_unique(where={"user_id": ...}): B723's row with no include
async fn read_creation_user(
    store: &PostgresStore,
    user_id: &str,
) -> Result<Option<LiteLLMUserTable>, OrgAccessError> {
    let session = store.session().await.map_err(OrgAccessError::Session)?;
    UserRepository {
        db: Some(&session),
        writer_db: None,
        use_writer: false,
    }
    .find_by_id(user_id)
    .await
    .map_err(OrgAccessError::UserRead)
}

#[derive(Serialize)]
struct DetailBody {
    detail: &'static str,
}

#[derive(Serialize)]
struct ErrorDetail {
    error: &'static str,
}

#[derive(Serialize)]
struct ErrorBody {
    detail: ErrorDetail,
}

#[derive(Serialize)]
struct InternalMessage {
    message: &'static str,
    #[serde(rename = "type")]
    error_type: &'static str,
}

#[derive(Serialize)]
struct InternalBody {
    error: InternalMessage,
}

impl IntoResponse for OrgAccessError {
    fn into_response(self) -> Response {
        match self {
            Self::Denied => (
                StatusCode::FORBIDDEN,
                Json(DetailBody {
                    detail: NO_ACCESS_DETAIL,
                }),
            )
                .into_response(),
            Self::DatabaseNotConnected => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    detail: ErrorDetail {
                        error: CommonProxyErrors::DbNotConnectedError.as_str(),
                    },
                }),
            )
                .into_response(),
            error @ (Self::Session(_)
            | Self::UserRead(_)
            | Self::MembershipRead(_)
            | Self::Record(_)) => {
                tracing::error!(error = %error, "the organization member lookup failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(InternalBody {
                        error: InternalMessage {
                            message: "Internal server error",
                            error_type: "internal_server_error",
                        },
                    }),
                )
                    .into_response()
            }
        }
    }
}
