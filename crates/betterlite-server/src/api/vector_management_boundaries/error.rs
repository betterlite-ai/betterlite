use betterlite_auth::auth_team_lookup::error::Detail;
use betterlite_core::api::vector_store_access::error::VectorStoreAccessError;
use betterlite_db::auxiliary_repository::repository::TableError;
use serde::Serialize;
use thiserror::Error;

// the 403 detail of _fetch_and_authorize_vector_store, shared with the info route's registry branch
pub const ACCESS_DENIED: &str =
    "Access denied: You do not have permission to access this vector store";

// the trailing text of the 400 detail _raise_if_config_owned builds around the id it owns
const CONFIG_OWNED_TAIL: &str = "is defined in the config file, so the config file owns it and it cannot be changed here. Edit the config file to change it, or remove it from the file to let the database own it.";

// _ConfigOwnedDetail: the two fields the config-owned 400 carries and nothing else
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConfigOwnedDetail {
    pub error: String,
    pub vector_store_id: String,
}

// LiteLLM's {"error": message} detail for the HTTP exceptions its lookups raise
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ErrorDetail {
    pub error: String,
}

// an HTTPException's detail: a plain string, {"error": message}, or the config-owned object
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum VectorManagementDetail {
    Text(String),
    Error(ErrorDetail),
    ConfigOwned(ConfigOwnedDetail),
}

#[derive(Debug, Error)]
pub enum VectorManagementError {
    #[error("vector store {vector_store_id} is owned by the config file")]
    ConfigOwned { vector_store_id: String },
    #[error("vector store {vector_store_id} was not found")]
    NotFound { vector_store_id: String },
    #[error("access to the vector store was denied")]
    AccessDenied,
    #[error(transparent)]
    Access(#[from] VectorStoreAccessError),
    #[error(transparent)]
    Rows(#[from] TableError),
    #[error("the stored JSON could not be projected")]
    Json(#[from] serde_json::Error),
}

impl VectorManagementError {
    // the HTTPException the helper raises; None is the plain exception the route turns into a 500
    #[must_use]
    pub fn http(&self) -> Option<(u16, VectorManagementDetail)> {
        match self {
            Self::ConfigOwned { vector_store_id } => Some((
                400,
                VectorManagementDetail::ConfigOwned(ConfigOwnedDetail {
                    error: format!("Vector store {vector_store_id} {CONFIG_OWNED_TAIL}"),
                    vector_store_id: vector_store_id.clone(),
                }),
            )),
            Self::NotFound { vector_store_id } => Some((
                404,
                VectorManagementDetail::Text(format!(
                    "Vector store with ID {vector_store_id} not found"
                )),
            )),
            Self::AccessDenied => {
                Some((403, VectorManagementDetail::Text(ACCESS_DENIED.to_owned())))
            }
            Self::Access(VectorStoreAccessError::Team(error)) => {
                error.http().map(|(status, kind)| {
                    let message = error.to_string();
                    let rendered = match kind {
                        Detail::Error => {
                            VectorManagementDetail::Error(ErrorDetail { error: message })
                        }
                        Detail::Text => VectorManagementDetail::Text(message),
                    };
                    (status, rendered)
                })
            }
            Self::Access(
                VectorStoreAccessError::Fields(_)
                | VectorStoreAccessError::Params(_)
                | VectorStoreAccessError::Limits(_),
            )
            | Self::Rows(_)
            | Self::Json(_) => None,
        }
    }
}
