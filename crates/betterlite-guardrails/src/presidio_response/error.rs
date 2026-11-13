use std::io::Error;

use thiserror::Error;

use crate::presidio_actions::error::PresidioActionsError;

#[derive(Debug, Error)]
pub enum PresidioResponseError {
    #[error("the request messages field must be a JSON array")]
    MessagesNotList,
    #[error("the request messages field does not hold message objects")]
    MessagesJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("a Presidio response body could not be read or written as JSON")]
    BodyJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Presidio logging hook could not start an async runtime")]
    Runtime {
        #[source]
        source: Error,
    },
    #[error("the Presidio logging hook could not build its own HTTP client")]
    Client {
        #[source]
        source: reqwest::Error,
    },
    #[error("the Presidio logging hook thread panicked")]
    HookThreadPanicked,
    #[error(transparent)]
    Actions(#[from] PresidioActionsError),
}

impl PresidioResponseError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::Actions(source) => source.status(),
            Self::MessagesNotList
            | Self::MessagesJson { .. }
            | Self::BodyJson { .. }
            | Self::Runtime { .. }
            | Self::Client { .. }
            | Self::HookThreadPanicked => 500,
        }
    }
}
