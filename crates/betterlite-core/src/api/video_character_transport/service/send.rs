use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_state::LoggingState;

use super::super::clients::{CharacterAsyncClient, CharacterRequest, CharacterSyncClient};
use super::super::config::CharacterConfig;
use super::super::error::{CharacterFault, converted, synthesized};
use super::wire::status_failure;
use crate::api::video_records::CharacterObject;

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

pub(crate) fn create_blocking<C, Client>(
    config: &C,
    logging: &mut LoggingState,
    client: &Client,
    request: CharacterRequest,
) -> Result<CharacterObject, CharacterFault<C>>
where
    C: CharacterConfig,
    Client: CharacterSyncClient,
{
    let reply = client
        .send_blocking(request)
        .map_err(|failure| converted(config, failure))?;
    checked(config, &reply)?;
    config
        .transform_create_response(&reply, logging)
        .map_err(|error| synthesized(config, &error))
}

pub(crate) fn get_blocking<C, Client>(
    config: &C,
    logging: &mut LoggingState,
    client: &Client,
    request: CharacterRequest,
) -> Result<CharacterObject, CharacterFault<C>>
where
    C: CharacterConfig,
    Client: CharacterSyncClient,
{
    let reply = client
        .send_blocking(request)
        .map_err(|failure| converted(config, failure))?;
    checked(config, &reply)?;
    config
        .transform_get_response(&reply, logging)
        .map_err(|error| synthesized(config, &error))
}

pub(crate) async fn create_async<C, Client>(
    config: &C,
    logging: &mut LoggingState,
    client: &Client,
    request: CharacterRequest,
) -> Result<CharacterObject, CharacterFault<C>>
where
    C: CharacterConfig,
    Client: CharacterAsyncClient,
{
    let reply = client
        .send(request)
        .await
        .map_err(|failure| converted(config, failure))?;
    checked(config, &reply)?;
    config
        .transform_create_response(&reply, logging)
        .map_err(|error| synthesized(config, &error))
}

pub(crate) async fn get_async<C, Client>(
    config: &C,
    logging: &mut LoggingState,
    client: &Client,
    request: CharacterRequest,
) -> Result<CharacterObject, CharacterFault<C>>
where
    C: CharacterConfig,
    Client: CharacterAsyncClient,
{
    let reply = client
        .send(request)
        .await
        .map_err(|failure| converted(config, failure))?;
    checked(config, &reply)?;
    config
        .transform_get_response(&reply, logging)
        .map_err(|error| synthesized(config, &error))
}
