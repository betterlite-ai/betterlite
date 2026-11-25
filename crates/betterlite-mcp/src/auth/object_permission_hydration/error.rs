use std::error::Error as StdError;

use thiserror::Error;

// LiteLLM answers HTTPException(503, "User policy is unavailable") when a fresh user-policy read fails
pub const USER_POLICY_UNAVAILABLE_STATUS: u16 = 503;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct UnloadableEntitlementError<Source>
where
    Source: StdError + 'static,
{
    message: String,
    #[source]
    cause: Option<Source>,
}

impl<Source> UnloadableEntitlementError<Source>
where
    Source: StdError + 'static,
{
    pub(crate) fn named(
        principal: &str,
        object_permission_id: &str,
        cause: Option<Source>,
    ) -> Self {
        Self {
            message: format!(
                "{principal} names object_permission_id {object_permission_id:?} which could not be loaded"
            ),
            cause,
        }
    }

    pub(crate) fn toolsets(toolset_ids: &[String]) -> Self {
        Self {
            message: format!("declared mcp_toolsets {toolset_ids:?} resolved to no grants"),
            cause: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum EntitlementError<Source>
where
    Source: StdError + 'static,
{
    #[error("the entitlement read failed: {source}")]
    Read {
        #[source]
        source: Source,
    },
    #[error(transparent)]
    Unloadable(#[from] UnloadableEntitlementError<Source>),
}

#[derive(Debug, Error)]
pub enum UserPermissionError<Source>
where
    Source: StdError + 'static,
{
    #[error("the user's object permission read failed: {source}")]
    Read {
        #[source]
        source: Source,
    },
    #[error(
        "user {user_id:?} names object_permission_id {object_permission_id:?} which could not be loaded"
    )]
    Missing {
        user_id: String,
        object_permission_id: String,
    },
    #[error("User policy is unavailable")]
    Unavailable {
        #[source]
        source: Source,
    },
}
