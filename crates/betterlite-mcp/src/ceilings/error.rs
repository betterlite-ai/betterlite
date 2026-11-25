use std::error::Error as StdError;

use thiserror::Error;

use super::reads::CeilingReads;
use crate::auth::object_permission_hydration::error::UnloadableEntitlementError;

// every read a ceiling makes fails either on an entitlement it knows exists or on any other fault
#[derive(Debug, Error)]
pub enum CeilingReadError<Source>
where
    Source: StdError + 'static,
{
    #[error(transparent)]
    Unloadable(UnloadableEntitlementError<Source>),
    #[error("{0}")]
    Failed(String),
}

// the fault a ceiling method answers with: an unloadable entitlement denies every caller shape, a named failure is the caller's decision
#[derive(Debug, Error)]
pub enum CeilingsError<Source>
where
    Source: StdError + 'static,
{
    #[error(transparent)]
    Unloadable(UnloadableEntitlementError<Source>),
    #[error("{0}")]
    Failed(String),
    #[error("MCP user ceiling unresolvable for user_id={user_id:?}")]
    UserCeilingUnresolvable { user_id: Option<String> },
}

pub type CeilingsResult<Value, Reads> =
    Result<Value, CeilingsError<<Reads as CeilingReads>::Error>>;

impl<Source> From<CeilingReadError<Source>> for CeilingsError<Source>
where
    Source: StdError + 'static,
{
    fn from(error: CeilingReadError<Source>) -> Self {
        match error {
            CeilingReadError::Unloadable(unloadable) => Self::Unloadable(unloadable),
            CeilingReadError::Failed(message) => Self::Failed(message),
        }
    }
}
