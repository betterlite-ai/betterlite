use std::time::Duration;

use betterlite_http::interactions::base::{ErrorClasses, RequestFailure, ResponseHeaders};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_state::LoggingState;

use super::super::config::CharacterConfig;
use super::super::error::{CharacterError, CharacterFault, converted, synthesized};
use super::wire::status_failure;
use crate::api::video_records::CharacterObject;
use crate::api::videos_http_transport::client::{ClientKind, HttpVerb, Sent};
use crate::api::videos_http_transport::error::elapsed_text;

// the response transform a call runs once its status has passed
pub(crate) type Transform<C> = fn(
    &C,
    &UpstreamResponse,
    &mut LoggingState,
) -> Result<CharacterObject, <C as ErrorClasses>::Error>;

// httpx's raise_for_status: only a 2xx answer reaches the response transform
fn checked<C>(config: &C, reply: &UpstreamResponse) -> Result<(), CharacterFault<C>>
where
    C: CharacterConfig,
{
    if (200..=299).contains(&reply.status) {
        return Ok(());
    }
    Err(converted(config, status_failure(reply)))
}

// the checked reply goes through its transform, and a transform failure takes _handle_error's synthesized 500
pub(crate) fn answered<C>(
    config: &C,
    logging: &mut LoggingState,
    reply: &UpstreamResponse,
    transform: Transform<C>,
) -> Result<CharacterObject, CharacterFault<C>>
where
    C: CharacterConfig,
{
    checked(config, reply)?;
    transform(config, reply, logging).map_err(|error| synthesized(config, &error))
}

// httpx.post turns its own timeout into litellm.Timeout, which _handle_error reports with the 408 it carries; a get raises none
pub(super) fn send_failure<C>(
    config: &C,
    sent: Sent,
    elapsed: Duration,
    failure: RequestFailure,
) -> CharacterFault<C>
where
    C: CharacterConfig,
{
    let timed_out = sent.verb == HttpVerb::Post
        && matches!(&failure, RequestFailure::Transport(source) if source.is_timeout());
    if !timed_out {
        return converted(config, failure);
    }
    let message = timeout_message(sent, elapsed);
    CharacterError::Provider(config.get_error_class(&message, 408_u16, &ResponseHeaders::new()))
}

// the message litellm.Timeout carries: the sync post names the timeout it was passed, the async one adds the time taken
fn timeout_message(sent: Sent, elapsed: Duration) -> String {
    let passed = sent.timeout.map_or_else(
        || String::from("None"),
        |value| value.as_secs_f64().to_string(),
    );
    match sent.mode {
        ClientKind::Sync => {
            format!("litellm.Timeout: Connection timed out after {passed} seconds.")
        }
        ClientKind::Asynchronous => format!(
            "litellm.Timeout: Connection timed out. Timeout passed={passed}, time taken={} seconds",
            elapsed_text(elapsed)
        ),
    }
}
