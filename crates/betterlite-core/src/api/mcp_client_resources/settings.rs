use std::time::{Duration, TryFromFloatSecsError};

use betterlite_config::sections::mcp_tool_helpers::Settings as McpToolHelperSettings;
use thiserror::Error;

#[derive(Clone, Copy, Debug)]
pub struct ResourceSettings {
    pub listing_timeout: Duration,
}

#[derive(Debug, Error)]
pub enum DeadlineError {
    #[error("the MCP tool listing timeout of {seconds} seconds is not a valid duration")]
    ListingTimeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
}

impl ResourceSettings {
    pub fn from_config(settings: &McpToolHelperSettings) -> Result<Self, DeadlineError> {
        let seconds = settings.tool_listing_timeout;
        let listing_timeout = Duration::try_from_secs_f64(seconds)
            .map_err(|source| DeadlineError::ListingTimeout { seconds, source })?;
        Ok(Self { listing_timeout })
    }
}
