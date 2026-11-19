mod backend;
mod notice;

use std::error::Error;

use betterlite_models::decisions::Json;
use futures_util::FutureExt as _;
use futures_util::future::BoxFuture;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_guardrails::error::RealtimeGuardrailsError;
use crate::api::realtime_guardrails::message;

use super::RealtimeGuardrails;

type UnitFuture<'call, ConfigError, CallbackError> =
    BoxFuture<'call, Result<(), RealtimeGuardrailsError<ConfigError, CallbackError>>>;

// The ordered steps of one block: a frame for the backend, or the client-only notice LiteLLM sends between them
enum BlockStep {
    Backend(String),
    Client,
}

fn block_steps(
    message: &Json,
    pre_block_backend_message: Option<&str>,
) -> Result<Vec<BlockStep>, serde_json::Error> {
    let (cancel, response_create) = message::control_frames();
    let mut steps = Vec::new();
    if let Some(text) = pre_block_backend_message {
        steps.push(BlockStep::Backend(text.to_owned()));
    }
    steps.push(BlockStep::Backend(serde_json::to_string(&cancel)?));
    steps.push(BlockStep::Client);
    steps.push(BlockStep::Backend(serde_json::to_string(
        &message::spoken_message(message),
    )?));
    steps.push(BlockStep::Backend(serde_json::to_string(&response_create)?));
    Ok(steps)
}

// The frame order: the caller's protocol frame first, the cancellation, the client notice, then the spoken message and a fresh response
pub(super) fn send_block_frames<'call, Config, CallbackError>(
    runner: &'call mut RealtimeGuardrails<'_, Config, CallbackError>,
    message: &'call Json,
    pre_block_backend_message: Option<&'call str>,
) -> UnitFuture<'call, Config::Error, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    async move {
        let steps = block_steps(message, pre_block_backend_message)
            .map_err(RealtimeGuardrailsError::Encode)?;
        for step in steps {
            match step {
                BlockStep::Backend(text) => backend::send_backend_text(&mut *runner, &text).await?,
                BlockStep::Client => notice::send_client_notice(&mut *runner, message).await?,
            }
        }
        Ok(())
    }
    .boxed()
}
