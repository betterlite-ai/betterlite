use std::collections::BTreeMap;
use std::future::Future;

use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use serde_json::Number;

use crate::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};

mod tests;

// LiteLLM fails a refused or unexpected read closed, so a bad credential is never retried as an outage
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadFault {
    Refused,
    DatabaseUnavailable { message: String },
    Other(String),
}

pub trait UserReads: Sync {
    fn has_database(&self) -> bool;

    fn user_by_id(
        &self,
        user_id: &str,
        fresh: bool,
    ) -> impl Future<Output = Result<Option<LiteLLMUserTable>, ReadFault>> + Send;

    fn object_permission(
        &self,
        object_permission_id: &str,
        fresh: bool,
    ) -> impl Future<Output = Result<Option<LiteLLMObjectPermissionTable>, ReadFault>> + Send;
}

pub trait TeamLimits: Sync {
    fn team_rpm_limits(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> impl Future<Output = Option<BTreeMap<String, BTreeMap<String, i64>>>> + Send;
}

pub trait DelegatedHuman: Sync {
    fn reload_admitted_user(
        &self,
        user_id: &str,
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<UserAPIKeyAuth, AdmittedUserError>> + Send;
}

pub struct AdmittedUserReload<U: UserReads, T: TeamLimits> {
    pub users: U,
    pub team_limits: T,
}

impl<U: UserReads, T: TeamLimits> AdmittedUserReload<U, T> {
    // clippy's same_name_method refuses reload_admitted_user while DelegatedHuman declares it
    pub async fn reload(
        &self,
        user_id: &str,
        requires_fresh_policy: bool,
    ) -> Result<UserAPIKeyAuth, AdmittedUserError> {
        if !self.users.has_database() {
            return Err(AdmittedUserError::NoDatabase);
        }
        let stored = self
            .users
            .user_by_id(user_id, requires_fresh_policy)
            .await
            .map_err(read_fault)?;
        let object_permission = match stored.as_ref() {
            Some(row) => self.permission_for(row, requires_fresh_policy).await?,
            None => None,
        };
        let Some(user) = stored else {
            return Err(AdmittedUserError::InvalidCredential);
        };
        if scim_deactivated(&user) {
            return Err(AdmittedUserError::InvalidCredential);
        }
        let fields =
            auth_fields(&user, object_permission.as_ref()).map_err(AdmittedUserError::Auth)?;
        let mut auth =
            UserAPIKeyAuth::construct(AuthInput::Dict(fields)).map_err(AdmittedUserError::Auth)?;
        auth.extra.mcp_access.mcp_admitted_user_subject = true;
        auth.extra.requires_fresh_policy = requires_fresh_policy;
        auth.extra.mcp_source_team_rpm_limits = self.team_limits.team_rpm_limits(&auth).await;
        Ok(auth)
    }

    async fn permission_for(
        &self,
        user: &LiteLLMUserTable,
        fresh: bool,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, AdmittedUserError> {
        if let Some(permission) = &user.object_permission {
            return Ok(Some(permission.clone()));
        }
        let Some(object_permission_id) = user.object_permission_id.as_deref() else {
            return Ok(None);
        };
        self.users
            .object_permission(object_permission_id, fresh)
            .await
            .map_err(read_fault)
    }
}

impl<U: UserReads, T: TeamLimits> DelegatedHuman for AdmittedUserReload<U, T> {
    fn reload_admitted_user(
        &self,
        user_id: &str,
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<UserAPIKeyAuth, AdmittedUserError>> + Send {
        self.reload(user_id, requires_fresh_policy)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AdmittedUserError {
    #[error("Server misconfigured: no database connection")]
    NoDatabase,
    #[error("Invalid or expired credential")]
    InvalidCredential,
    #[error("{message}")]
    DatabaseUnavailable { message: String },
    // LiteLLM builds this auth outside its try block, so the proxy answers a bad row as an unhandled 500
    #[error("{0}")]
    Auth(#[source] AuthError),
}

impl AdmittedUserError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::NoDatabase | Self::Auth(_) => 500,
            Self::InvalidCredential => 401,
            Self::DatabaseUnavailable { .. } => 503,
        }
    }
}

#[must_use]
pub fn with_session_resource_server_id(
    mut auth: UserAPIKeyAuth,
    resource_server_id: Option<String>,
) -> UserAPIKeyAuth {
    auth.extra.mcp_session_resource_server_id = resource_server_id;
    auth
}

fn read_fault(fault: ReadFault) -> AdmittedUserError {
    match fault {
        ReadFault::Refused | ReadFault::Other(_) => AdmittedUserError::InvalidCredential,
        ReadFault::DatabaseUnavailable { message } => {
            AdmittedUserError::DatabaseUnavailable { message }
        }
    }
}

fn scim_deactivated(user: &LiteLLMUserTable) -> bool {
    user.metadata
        .as_ref()
        .and_then(|metadata| metadata.0.get("scim_active"))
        .is_some_and(|value| matches!(value, JsonData::Bool(false)))
}

fn auth_fields(
    user: &LiteLLMUserTable,
    object_permission: Option<&LiteLLMObjectPermissionTable>,
) -> Result<BTreeMap<String, JsonData>, AuthError> {
    let mut fields = row_fields(user);
    fields.insert(
        "object_permission".to_owned(),
        permission_json(object_permission)?,
    );
    Ok(fields)
}

fn row_fields(user: &LiteLLMUserTable) -> BTreeMap<String, JsonData> {
    let mut fields = BTreeMap::new();
    fields.insert("user_id".to_owned(), JsonData::String(user.user_id.clone()));
    fields.insert(
        "user_role".to_owned(),
        user.user_role
            .clone()
            .map_or(JsonData::Null, JsonData::String),
    );
    fields.insert(
        "org_id".to_owned(),
        user.organization_id
            .clone()
            .map_or(JsonData::Null, JsonData::String),
    );
    fields.insert(
        "object_permission_id".to_owned(),
        user.object_permission_id
            .clone()
            .map_or(JsonData::Null, JsonData::String),
    );
    fields.insert(
        "user_tpm_limit".to_owned(),
        user.tpm_limit.map_or(JsonData::Null, |limit| {
            JsonData::Number(Number::from(limit))
        }),
    );
    fields.insert(
        "user_rpm_limit".to_owned(),
        user.rpm_limit.map_or(JsonData::Null, |limit| {
            JsonData::Number(Number::from(limit))
        }),
    );
    fields
}

fn permission_json(
    permission: Option<&LiteLLMObjectPermissionTable>,
) -> Result<JsonData, AuthError> {
    let Some(object_permission) = permission else {
        return Ok(JsonData::Null);
    };
    serde_json::to_string(object_permission)
        .and_then(|text| serde_json::from_str(&text))
        .map_err(AuthError::Json)
}
