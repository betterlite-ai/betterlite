use std::error::Error as StdError;
use std::fmt::Display;

use super::error::ProviderClass;
use crate::realtime_bridge::end::ForwardEnd;
use crate::realtime_bridge::error::{ClientForwardError, ProviderForwardError};
use crate::realtime_events::ack::AckError;
use crate::realtime_events::failure::ProviderFailure;
use crate::realtime_events::stream::StreamError;

pub(super) fn provider_failure<E>(
    client: Result<ForwardEnd, ClientForwardError<E>>,
    provider: Result<ForwardEnd, ProviderForwardError<E>>,
) -> Option<ProviderFailure<E>>
where
    E: StdError + 'static + ProviderClass,
{
    // The client direction's exception wins over the provider direction's, as the source reports it first
    if let Err(error) = client {
        return Some(match error {
            ClientForwardError::Send(error) => stream_failure(error),
            ClientForwardError::Ack(AckError::Stream(error)) => stream_failure(error),
            ClientForwardError::Input(error) => named_failure("InputError", &error),
            ClientForwardError::Ack(error) => named_failure("AckError", &error),
            ClientForwardError::Encode { source } => named_failure("EncodeError", &source),
        });
    }
    match provider {
        Err(error) => Some(match error {
            ProviderForwardError::Stream(error) => stream_failure(error),
            ProviderForwardError::Utf8 { source } => named_failure("UnicodeDecodeError", &source),
            ProviderForwardError::Transform(error) => named_failure("EventError", &error),
        }),
        Ok(_) => None,
    }
}

fn stream_failure<E>(error: StreamError<E>) -> ProviderFailure<E>
where
    E: StdError + 'static + ProviderClass,
{
    let StreamError { message, source } = error;
    let class_name = source
        .as_ref()
        .and_then(|error| error.provider_class_name().map(str::to_owned))
        .unwrap_or_else(|| "RuntimeError".to_owned());
    let message = source
        .as_ref()
        .and_then(|error| error.provider_message().map(str::to_owned))
        .unwrap_or(message);
    ProviderFailure {
        class_name,
        message,
        source,
    }
}

fn named_failure<E>(class_name: &str, error: &dyn Display) -> ProviderFailure<E> {
    ProviderFailure {
        class_name: class_name.to_owned(),
        message: error.to_string(),
        source: None,
    }
}
