use std::error::Error;

use futures_util::FutureExt as _;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use crate::api::realtime_guardrails::error::RealtimeGuardrailsError;
use crate::api::realtime_sender::RealtimeSend as _;
use crate::api::realtime_sender::RealtimeSender;

use super::{RealtimeGuardrails, UnitFuture};

// B1110's send boundary: a guardrail frame gets the same provider transform as a client frame
pub(super) fn send_backend_text<'call, Config, CallbackError>(
    runner: &'call mut RealtimeGuardrails<'_, Config, CallbackError>,
    text: &'call str,
) -> UnitFuture<'call, Config::Error, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + 'static,
{
    async move {
        let guardrails: &[&dyn RealtimeGuardrailCallback<Error = CallbackError>] = &[];
        let mut sender = RealtimeSender {
            state: &mut *runner.state,
            provider: runner.provider,
            guardrails,
        };
        sender
            .send_to_backend(text)
            .await
            .map_err(RealtimeGuardrailsError::Send)?;
        Ok(())
    }
    .boxed_local()
}
