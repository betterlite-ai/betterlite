use async_trait::async_trait;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::table_ops::rows::RowDecoder as _;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use serde::Serialize;

use crate::organization_create::caller::OrgCaller;

pub const NO_ACCESS_DETAIL: &str = "You do not have access to this organization";
pub const ORG_ADMIN_ROLE: &str = "org_admin";
// B723's user_unique projection: the stored row plus the membership rows get_user_object includes
const USER_WITH_MEMBERSHIPS_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT u.*, COALESCE((SELECT json_agg(m.*) FROM \"LiteLLM_OrganizationMembership\" m WHERE m.\"user_id\" = u.\"user_id\"), '[]'::json) AS organization_memberships FROM \"LiteLLM_UserTable\" u WHERE u.\"user_id\" = $1) x";

#[derive(Debug, thiserror::Error)]
pub enum OrgAccessError {
    #[error("You do not have access to this organization")]
    Denied,
    #[error("the database is not connected")]
    DatabaseNotConnected,
    #[error("the caller lookup failed: {0}")]
    Lookup(String),
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
            .map_err(|error| OrgAccessError::Lookup(error.to_string()))?;
        let rows = session
            .query(USER_WITH_MEMBERSHIPS_SQL, &[&user_id])
            .await
            .map_err(|error| OrgAccessError::Lookup(error.to_string()))?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let text: String = row
            .try_get(0)
            .map_err(|error| OrgAccessError::Lookup(error.to_string()))?;
        LiteLLMUserTable::decode_row(&text)
            .map(Some)
            .map_err(|error| OrgAccessError::Lookup(error.to_string()))
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
    match (DatabaseUserLookup { store }).find_user(user_id).await {
        Ok(found) => found,
        Err(error) => {
            tracing::debug!(error = %error, "the caller user row could not be read");
            None
        }
    }
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
            Self::Lookup(message) => {
                tracing::error!(error = %message, "the organization member lookup failed");
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
