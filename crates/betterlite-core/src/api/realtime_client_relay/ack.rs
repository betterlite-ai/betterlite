use std::error::Error;

use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_sender::RealtimeSend as _;
use crate::api::realtime_session_state::logging::SessionInput;
use crate::api::realtime_setup_buffer::{SetupBuffer as _, SetupFlush as _};

use super::RealtimeClientRelay;
use super::error::RealtimeClientRelayError;
use super::rewrite;
use super::session;

const SESSION_UPDATE: &str = "session.update";
const RESPONSE_CREATE: &str = "response.create";

// One client frame: guardrail rewrites, the pending-response drop, the session rewrites, then logging and delivery
pub(super) async fn handle<Config, CallbackError>(
    relay: &mut RealtimeClientRelay<'_, Config, CallbackError>,
    message: &str,
) -> Result<(), RealtimeClientRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    let Some(mut event) = rewrite::ClientEvent::parse(message) else {
        relay.state.store_input(SessionInput::Text(message));
        return deliver(relay, None, message, false).await;
    };
    if rewrite::client_guardrails(relay, &mut event).await? {
        return Ok(());
    }
    if event.msg_type.as_deref() == Some(RESPONSE_CREATE)
        && relay.state.setup.pending_guardrail_message.is_some()
    {
        relay.state.setup.pending_guardrail_message = None;
        return Ok(());
    }
    let injected = if event.msg_type.as_deref() == Some(SESSION_UPDATE) {
        session::session_update(relay, &mut event)?
    } else {
        false
    };
    relay.state.store_input(SessionInput::Text(&event.message));
    deliver(relay, event.msg_type.as_deref(), &event.message, injected).await
}

// LiteLLM's buffer order: pre-setup audio queues, a backlog lets the setup through first, and a flush blocks new sends
async fn deliver<Config, CallbackError>(
    relay: &mut RealtimeClientRelay<'_, Config, CallbackError>,
    msg_type: Option<&str>,
    message: &str,
    injected: bool,
) -> Result<(), RealtimeClientRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    if relay.state.should_buffer_until_setup(message)? {
        relay.state.buffer_pending_message(message)?;
        return Ok(());
    }
    if !relay.state.setup.pending_messages_until_setup.is_empty() {
        let setup_first = !relay.state.setup.backend_setup_complete.0
            && !relay.state.setup.flushing_pending_messages.0
            && msg_type == Some(SESSION_UPDATE);
        if !setup_first {
            relay.state.buffer_pending_message(message)?;
            if relay.state.setup.backend_setup_complete.0
                && !relay.state.setup.flushing_pending_messages.0
            {
                let _flushed = relay.sender().flush_pending_messages().await?;
            }
            return Ok(());
        }
    }
    if relay.state.setup.flushing_pending_messages.0 {
        relay.state.buffer_pending_message(message)?;
        return Ok(());
    }
    let sent = relay.sender().send_to_backend(message).await?;
    if injected && sent {
        relay.state.setup.turn_detection_update_sent.0 = true;
    }
    Ok(())
}
