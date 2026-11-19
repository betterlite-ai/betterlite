use std::error::Error as StdError;

use thiserror::Error;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::credentials::error::CredentialError;
use crate::realtime_completion::error::CompletionError;
use crate::realtime_events::client::ClientError;
use crate::realtime_events::failure::ProviderStreamError;
use crate::realtime_events::scope::SessionCommitted;
use crate::realtime_events::stream::StreamError;

pub const NO_CREDENTIALS_MESSAGE: &str = "No AWS credentials found for Bedrock realtime. Set aws_* params in litellm_params or configure credentials in the environment";

// The class name a provider exception would carry: the SDK error names its own, a closed stream has none
pub trait ProviderClass {
    fn provider_class_name(&self) -> Option<&str>;

    fn provider_message(&self) -> Option<&str>;
}

// The owned preparation, connection and session failures; a provider failure arrives already mapped
#[derive(Debug, Error)]
pub enum RealtimeTransportError<E, C>
where
    E: StdError + 'static,
    C: StdError + 'static,
{
    #[error(transparent)]
    Committed(#[from] SessionCommitted),
    #[error(transparent)]
    Endpoint(#[from] AwsEndpointError),
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("{message}")]
    NoCredentials { message: String, status_code: u16 },
    #[error(transparent)]
    Client(#[from] ClientError<C>),
    #[error("the session.created event does not encode as JSON")]
    SessionCreated {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Stream(#[from] StreamError<E>),
    #[error(transparent)]
    Completion(#[from] CompletionError),
    #[error("a logged realtime event does not convert to a logging record")]
    Logging {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Provider(#[from] ProviderStreamError<E>),
}

impl<E, C> RealtimeTransportError<E, C>
where
    E: StdError + 'static,
    C: StdError + 'static,
{
    // The status an HTTP layer answers with; a local failure has none
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Committed(SessionCommitted { status_code })
            | Self::NoCredentials { status_code, .. }
            | Self::Provider(ProviderStreamError::Bedrock { status_code, .. }) => {
                Some(*status_code)
            }
            Self::Endpoint(_)
            | Self::Credential(_)
            | Self::Client(_)
            | Self::SessionCreated { .. }
            | Self::Stream(_)
            | Self::Completion(_)
            | Self::Logging { .. }
            | Self::Provider(ProviderStreamError::Unchanged(_)) => None,
        }
    }
}
