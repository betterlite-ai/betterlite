use axum::http::StatusCode;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::resource_rows::ResourceRowsError;

use super::model_info::RouterWeightModelInfoError;
use super::weights::RouterWeightsError;

#[derive(Debug, thiserror::Error)]
pub enum RouterWeightError {
    #[error("Invalid router weights. Replace or clear router_settings.weights.")]
    InvalidSettings(#[source] RouterWeightsError),
    #[error("Database unavailable while validating router weights")]
    DatabaseUnavailable,
    #[error("Unknown deployment ID in router weights: {deployment_id}")]
    UnknownDeployment { deployment_id: String },
    #[error("Deployment {deployment_id} does not belong to model group {model_group}")]
    WrongModelGroup {
        deployment_id: String,
        model_group: String,
    },
    #[error(transparent)]
    Stored(#[from] ResourceRowsError),
    #[error(transparent)]
    ModelInfo(#[from] RouterWeightModelInfoError),
    #[error(transparent)]
    Database(#[from] TableError),
}

impl RouterWeightError {
    #[must_use]
    pub const fn status_code(&self) -> Option<StatusCode> {
        match self {
            Self::InvalidSettings(_)
            | Self::UnknownDeployment { .. }
            | Self::WrongModelGroup { .. } => Some(StatusCode::BAD_REQUEST),
            Self::DatabaseUnavailable => Some(StatusCode::SERVICE_UNAVAILABLE),
            Self::Stored(_) | Self::ModelInfo(_) | Self::Database(_) => None,
        }
    }
}

impl From<RouterWeightsError> for RouterWeightError {
    fn from(source: RouterWeightsError) -> Self {
        Self::InvalidSettings(source)
    }
}
