use betterlite_http::interactions::base::ErrorClasses;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_state::LoggingState;

use super::super::config::CharacterConfig;
use super::super::error::{CharacterFault, converted, synthesized};
use super::wire::status_failure;
use crate::api::video_records::CharacterObject;

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
    reply: UpstreamResponse,
    transform: Transform<C>,
) -> Result<CharacterObject, CharacterFault<C>>
where
    C: CharacterConfig,
{
    checked(config, &reply)?;
    transform(config, &reply, logging).map_err(|error| synthesized(config, &error))
}
