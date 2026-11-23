use betterlite_db::key_repository::decode::TokenDecodeError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use std::collections::BTreeMap;
use std::convert::Infallible;

use crate::key_access::error::{DetailShape, KeyAccessError};
use crate::key_fields::error::DecodeError;
use crate::key_policy_fields::ClockError;
use crate::key_policy_fields::columns::ColumnError;
use crate::key_policy_fields::model_budget::ModelBudgetError;
use crate::object_permissions::error::ObjectPermissionError;
use crate::router_weight_references::error::RouterWeightError;

// The failures the owned key-patch helpers raise, each carrying the HTTP answer LiteLLM gives for it.
#[derive(Debug, thiserror::Error)]
pub enum KeyPatchError {
    #[error("{field} is immutable once set and cannot be changed via update.")]
    ImmutableMetadata { field: String },
    #[error(
        "team_id is required for service account keys. Please specify `team_id` in the request body."
    )]
    ServiceAccountTeamRequired,
    #[error("{field} must be a non-negative finite number. Received: {received}")]
    NonFiniteBudget {
        field: &'static str,
        received: String,
    },
    #[error("Key not found.")]
    KeyNotFound,
    #[error("Database not connected")]
    DatabaseNotConnected,
    #[error("{0}")]
    LoggingMetadata(#[source] Box<KeyAccessError>),
    #[error(transparent)]
    RouterWeights(#[from] RouterWeightError),
    #[error(transparent)]
    ModelBudget(#[from] ModelBudgetError),
    #[error(transparent)]
    ObjectPermission(#[from] ObjectPermissionError),
    #[error(transparent)]
    Clock(#[from] ClockError),
    #[error(transparent)]
    Column(#[from] ColumnError),
    #[error("the key request could not be projected")]
    Request(#[from] DecodeError),
    #[error("the stored key row could not be decoded")]
    StoredRow(#[from] TokenDecodeError),
    #[error("the stored key row is not a JSON object")]
    StoredRowShape,
    #[error("{name} is not a key column")]
    UnknownColumn { name: String },
    #[error("general_settings is not a JSON object")]
    GeneralSettings,
    #[error(transparent)]
    Database(#[from] DbError<Infallible>),
    #[error("the key patch could not be encoded")]
    Json(#[from] serde_json::Error),
}

impl KeyPatchError {
    // The status and detail shape of the HTTPException the source raises; None is an exception no handler maps.
    #[must_use]
    pub fn response(&self) -> (Option<u16>, Option<DetailShape>) {
        match self {
            Self::ImmutableMetadata { .. } | Self::ServiceAccountTeamRequired => {
                (Some(400_u16), Some(DetailShape::Text))
            }
            Self::NonFiniteBudget { .. } | Self::LoggingMetadata(_) => {
                (Some(400_u16), Some(DetailShape::Error))
            }
            Self::DatabaseNotConnected => (Some(500_u16), Some(DetailShape::Error)),
            Self::KeyNotFound => (Some(404_u16), Some(DetailShape::Text)),
            Self::RouterWeights(error) => (
                error.status_code().map(|code| code.as_u16()),
                Some(DetailShape::Text),
            ),
            Self::ModelBudget(_)
            | Self::ObjectPermission(_)
            | Self::Clock(_)
            | Self::Column(_)
            | Self::Request(_)
            | Self::StoredRow(_)
            | Self::StoredRowShape
            | Self::UnknownColumn { .. }
            | Self::GeneralSettings
            | Self::Database(_)
            | Self::Json(_) => (None, None),
        }
    }

    // Only a missing key raises the ProxyException shape; every other failure is an HTTPException or a 500.
    #[must_use]
    pub fn proxy_exception(&self) -> Option<ProxyException> {
        let Self::KeyNotFound = self else {
            return None;
        };
        Some(ProxyException::new(ProxyExceptionInit {
            message: String::from("Key not found."),
            error_type: ProxyErrorTypes::NotFoundError.to_string(),
            param: Some(String::from("key")),
            code: ExceptionCode::Status(404),
            headers: BTreeMap::new(),
            openai_code: None,
            provider_specific_fields: BTreeMap::new(),
        }))
    }
}
