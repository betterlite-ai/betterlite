pub mod io_token;
pub mod scope_targets;
pub mod success_event;

mod io_token_ops_early;
mod io_token_ops_input;
mod io_token_released;
mod io_token_split;
mod io_token_usage;
mod io_token_zero;
mod scope_format;
mod scope_order;
mod scope_presence;
mod success_ops_emission;
mod success_ops_exempt;
mod success_ops_filtering;
mod success_ops_logging;
mod success_ops_model;
mod success_ops_scopes;
mod success_ops_tokens;

mod tests;

use crate::model_descriptors::ModelDescriptorsError;
use crate::token_adjustments::store::TokenAdjustmentError;

#[derive(Debug, thiserror::Error)]
pub enum ReconciliationError {
    #[error(transparent)]
    Tokens(#[from] TokenAdjustmentError),
    #[error(transparent)]
    Descriptors(#[from] ModelDescriptorsError),
    #[error("the request metadata field {field} holds a value that is not text")]
    MetadataField { field: &'static str },
}

pub(crate) fn token_count(value: u64) -> i64 {
    if let Ok(count) = i64::try_from(value) {
        return count;
    }
    i64::MAX
}
