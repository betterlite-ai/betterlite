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
            ClientForwardError::Send(inner) | ClientForwardError::Ack(AckError::Stream(inner)) => {
                stream_failure(inner)
            }
            ClientForwardError::Input(inner) => named_failure("InputError", &inner),
            ClientForwardError::Ack(inner) => named_failure("AckError", &inner),
            ClientForwardError::Encode { source } => named_failure("EncodeError", &source),
        });
    }
    match provider {
        Err(error) => Some(match error {
            ProviderForwardError::Stream(inner) => stream_failure(inner),
            ProviderForwardError::Utf8 { source } => named_failure("UnicodeDecodeError", &source),
            ProviderForwardError::Transform(inner) => named_failure("EventError", &inner),
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
        .and_then(|cause| cause.provider_class_name().map(str::to_owned))
        .unwrap_or_else(|| "RuntimeError".to_owned());
    let text = source
        .as_ref()
        .and_then(|cause| cause.provider_message().map(str::to_owned))
        .unwrap_or(message);
    ProviderFailure {
        class_name,
        message: text,
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
