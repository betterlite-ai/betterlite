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

// the PTU refusals are bare HTTPExceptions in LiteLLM, so they carry no type or param
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub status: u16,
    pub error_type: Option<ProxyErrorTypes>,
    pub param: Option<String>,
}

impl ModelFieldError {
    #[must_use]
    pub fn refusal(&self) -> Refusal {
        let validation = Some(ProxyErrorTypes::ValidationError);
        let (status, error_type, param) = match self {
            Self::ConfigWrite(_) | Self::Placement(_) | Self::RouterModel(_) => {
                (400_u16, validation, Some("litellm_params.model".to_owned()))
            }
            Self::EmptyCredentialName | Self::CredentialNotFound { .. } => (
                400_u16,
                validation,
                Some("litellm_credential_name".to_owned()),
            ),
            Self::RateLimitsMissing { missing } => (
                400_u16,
                validation,
                missing
                    .first()
                    .map(|field| format!("litellm_params.{field}")),
            ),
            Self::PtuDisabled { .. } | Self::PtuInvalid { .. } => (400_u16, None, None),
            Self::CredentialLookup { .. }
            | Self::Database(_)
            | Self::Table(_)
            | Self::Row { .. } => (500_u16, None, None),
        };
        Refusal {
            status,
            error_type,
            param,
        }
    }
}
