use std::error::Error as StdError;
use std::str::Utf8Error;

use thiserror::Error;

use crate::realtime_events::ack::AckError;
use crate::realtime_events::error::EventError;
use crate::realtime_events::stream::StreamError;
use crate::realtime_input::error::InputError;

// The client pump forwards what the transforms return; every failure it can see is one of theirs
#[derive(Debug, Error)]
pub enum ClientForwardError<S>
where
    S: StdError + 'static,
{
    #[error(transparent)]
    Input(#[from] InputError),
    #[error(transparent)]
    Ack(#[from] AckError<S>),
    #[error(transparent)]
    Send(#[from] StreamError<S>),
    #[error("a Bedrock input frame does not encode as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

// A provider failure escapes the pump unconverted; the bridge maps it once both directions have stopped
#[derive(Debug, Error)]
pub enum ProviderForwardError<S>
where
    S: StdError + 'static,
{
    #[error(transparent)]
    Stream(#[from] StreamError<S>),
    #[error("the provider event bytes are not UTF-8")]
    Utf8 {
        #[source]
        source: Utf8Error,
    },
    #[error(transparent)]
    Transform(#[from] EventError),
}
