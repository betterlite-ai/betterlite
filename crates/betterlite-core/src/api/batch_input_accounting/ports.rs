use std::error::Error;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_models::model_registration::JsonObject;

use super::error::{BatchHttpError, BatchManagedFileError};
use crate::api::request_destination::RequestData;

// What a file read answered: bytes, content that is not bytes, a hook that cannot serve it, or an HTTPException
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchFileFetch {
    Content(Vec<u8>),
    NotBytes { kind: String },
    ManagedUnavailable(BatchManagedFileError),
    Http(BatchHttpError),
}

// The one read this slice makes: the managed files hook keeps the caller, the provider read takes fetch params
#[derive(Clone, Copy)]
pub enum BatchFileRequest<'fetch> {
    Managed {
        file_id: &'fetch str,
        caller: &'fetch UserAPIKeyAuth,
    },
    Provider {
        file_id: &'fetch str,
        params: &'fetch JsonObject,
        caller: Option<&'fetch UserAPIKeyAuth>,
    },
}

#[async_trait]
pub trait BatchFileSource: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn fetch(&self, request: BatchFileRequest<'_>) -> Result<BatchFileFetch, Self::Error>;
}

// The provider file id and fetch kwargs _resolve_batch_input_file_fetch_params answers
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchFetchParams {
    pub file_id: String,
    pub params: JsonObject,
}

// The descriptor service's row estimate and no-max-tokens floor, plus the provider fetch-params resolver
pub trait BatchAccountingCalls: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn output_floor(&self, min_configured_otpm_limit: Option<i64>) -> i64;

    fn estimate_output_tokens(
        &self,
        entry: &RequestData,
        min_configured_otpm_limit: Option<i64>,
    ) -> Result<i64, Self::Error>;

    fn resolve_fetch_params(
        &self,
        file_id: &str,
        custom_llm_provider: &str,
        data: &RequestData,
    ) -> Result<BatchFetchParams, Self::Error>;
}

// get_team_object: a loaded team, a failure it wraps in a 403, or an HTTPException it re-raises
#[derive(Debug)]
pub enum BatchTeamLookup<Team> {
    Loaded(Team),
    Failed,
    Http(BatchHttpError),
}

// can_team_access_model, _check_team_member_model_access and can_key_call_model's outcomes
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchAccessCheck {
    Allowed,
    Denied { message: String },
    Failed { message: String },
    Http(BatchHttpError),
}

// _key_access_group_grants_model: the grant that can lift a team denial
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchKeyGrant {
    Granted,
    Denied,
    Failed { message: String },
    Http(BatchHttpError),
}

// The key and team model checks _enforce_batch_file_model_access reuses from the auth slice
#[async_trait]
pub trait BatchModelAccess: Send + Sync {
    type Team: Send + Sync + 'static;
    type Error: Error + Send + Sync + 'static;

    fn database_available(&self) -> bool;

    async fn team(&self, team_id: &str) -> Result<BatchTeamLookup<Self::Team>, Self::Error>;

    async fn team_can_access(
        &self,
        model: &str,
        caller: &UserAPIKeyAuth,
        team: &Self::Team,
    ) -> Result<BatchAccessCheck, Self::Error>;

    async fn key_group_grants(
        &self,
        model: &str,
        caller: &UserAPIKeyAuth,
        team: &Self::Team,
    ) -> Result<BatchKeyGrant, Self::Error>;

    async fn member_can_access(
        &self,
        model: &str,
        caller: &UserAPIKeyAuth,
        team: &Self::Team,
    ) -> Result<BatchAccessCheck, Self::Error>;

    async fn key_can_call(
        &self,
        model: &str,
        caller: &UserAPIKeyAuth,
    ) -> Result<BatchAccessCheck, Self::Error>;
}
