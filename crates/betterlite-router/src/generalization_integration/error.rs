use betterlite_models::error::ModelsError;
use serde::{Deserialize, Serialize};

use crate::tag_matchers::patterns::WarningKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizationWarning {
    pub kind: WarningKind,
    pub pattern: String,
}

#[derive(Debug, thiserror::Error)]
pub enum GeneralizationError {
    #[error(transparent)]
    Rules(#[from] ModelsError),
    #[error("the configured generalization rules do not serialize: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
}
