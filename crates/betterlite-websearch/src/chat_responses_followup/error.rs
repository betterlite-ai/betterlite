use thiserror::Error;

use crate::call_transformation::TransformationError;

#[derive(Debug, Error)]
pub enum FollowupError {
    #[error("cannot build the search follow-up transformation")]
    Transformation(#[from] TransformationError),
    #[error("cannot encode a follow-up message")]
    Json(#[from] serde_json::Error),
}
