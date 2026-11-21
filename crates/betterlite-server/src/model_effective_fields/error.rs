use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_models::ptu_pricing::PTU_COST_ATTRIBUTION_ENV_VAR;
use thiserror::Error;

use crate::credentials::store::CredentialStoreError;
use crate::router_dependencies::error::{
    ComplexityConfigWriteError, ComplexityPlacementError, StrategyRouterWriteError,
};

#[derive(Debug, Error)]
pub enum ModelFieldError {
    #[error(transparent)]
    ConfigWrite(#[from] ComplexityConfigWriteError),
    #[error(transparent)]
    Placement(#[from] ComplexityPlacementError),
    #[error(transparent)]
    RouterModel(#[from] StrategyRouterWriteError),
    #[error(
        "litellm_credential_name cannot be an empty string. Send null to detach the stored \
         credential or omit the field to leave it unchanged."
    )]
    EmptyCredentialName,
    #[error(
        "Credential '{credential_name}' not found. Create it via /credentials before attaching it \
         to a model."
    )]
    CredentialNotFound { credential_name: String },
    #[error("the credential store could not be read")]
    CredentialLookup {
        #[source]
        source: CredentialStoreError,
    },
    #[error(
        "{} must be set to a positive value when 'enforce_rpm_tpm_on_model_add' is enabled in \
         general_settings",
        .missing.join(" and ")
    )]
    RateLimitsMissing { missing: Vec<&'static str> },
    #[error(
        "PTU cost attribution is disabled, so {} cannot be set. Set \
         {PTU_COST_ATTRIBUTION_ENV_VAR}=true to enable it.",
        .supplied.join(", ")
    )]
    PtuDisabled { supplied: Vec<&'static str> },
    #[error("{message}")]
    PtuInvalid { message: String },
    #[error("the model table is not reachable")]
    Database(#[from] DbError),
    #[error("the model row could not be read")]
    Table(#[from] TableError),
    #[error("the stored model row is not a deployment")]
    Row {
        #[source]
        source: serde_json::Error,
    },
}

impl ModelFieldError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::ConfigWrite(_)
            | Self::Placement(_)
            | Self::RouterModel(_)
            | Self::EmptyCredentialName
            | Self::CredentialNotFound { .. }
            | Self::RateLimitsMissing { .. }
            | Self::PtuDisabled { .. }
            | Self::PtuInvalid { .. } => 400,
            Self::CredentialLookup { .. }
            | Self::Database(_)
            | Self::Table(_)
            | Self::Row { .. } => 500,
        }
    }

    // the PTU refusals are bare HTTPExceptions in LiteLLM, so they carry no type, param or code
    #[must_use]
    pub const fn error_type(&self) -> Option<ProxyErrorTypes> {
        match self {
            Self::ConfigWrite(_)
            | Self::Placement(_)
            | Self::RouterModel(_)
            | Self::EmptyCredentialName
            | Self::CredentialNotFound { .. }
            | Self::RateLimitsMissing { .. } => Some(ProxyErrorTypes::ValidationError),
            Self::PtuDisabled { .. }
            | Self::PtuInvalid { .. }
            | Self::CredentialLookup { .. }
            | Self::Database(_)
            | Self::Table(_)
            | Self::Row { .. } => None,
        }
    }

    #[must_use]
    pub fn param(&self) -> Option<String> {
        match self {
            Self::ConfigWrite(_) | Self::Placement(_) | Self::RouterModel(_) => {
                Some("litellm_params.model".to_owned())
            }
            Self::EmptyCredentialName | Self::CredentialNotFound { .. } => {
                Some("litellm_credential_name".to_owned())
            }
            Self::RateLimitsMissing { missing } => missing
                .first()
                .map(|field| format!("litellm_params.{field}")),
            Self::PtuDisabled { .. }
            | Self::PtuInvalid { .. }
            | Self::CredentialLookup { .. }
            | Self::Database(_)
            | Self::Table(_)
            | Self::Row { .. } => None,
        }
    }
}
