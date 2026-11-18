use thiserror::Error;

use crate::routing_state_store::error::StateStoreError;
use crate::routing_value_store::StoreError;

#[derive(Debug, Error)]
pub enum RoutingStateError {
    #[error("the routing store failed a batched increment")]
    Store {
        #[from]
        source: StateStoreError,
    },
    #[error("the in-memory routing store failed: {source}")]
    Local {
        #[from]
        source: StoreError,
    },
    #[error("the routing store returned a value that is not a finite number")]
    NonFinite,
}
