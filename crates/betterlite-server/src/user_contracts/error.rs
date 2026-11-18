use betterlite_logging::private_json::error::PrivateJsonError;

use crate::proxy_request_contracts::ContractError;

#[derive(Debug, thiserror::Error)]
pub enum UserContractsError {
    #[error(transparent)]
    Bounded(#[from] PrivateJsonError),
    #[error(transparent)]
    Contract(#[from] ContractError),
}
