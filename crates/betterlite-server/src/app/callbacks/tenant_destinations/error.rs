use serde_json::Error as JsonError;
use thiserror::Error;

// the resolver reads fields off the auth's serde form; only that projection can fail
#[derive(Debug, Error)]
pub enum TenantDestinationError {
    #[error("the authenticated key cannot be read for tenant destinations: {0}")]
    Principal(#[source] JsonError),
}
