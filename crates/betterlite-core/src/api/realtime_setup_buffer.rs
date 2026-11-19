use std::error::Error;

use async_trait::async_trait;
use betterlite_models::decisions::Json;

use super::realtime_contracts::config::BaseRealtimeConfig;
use super::realtime_contracts::decode::{RealtimeDecodeLimits, decode_realtime_frame};
use super::realtime_contracts::error::RealtimeContractError;
use super::realtime_sender::{RealtimeSend as _, RealtimeSender};
use super::realtime_session_state::RealtimeStreaming;
use super::realtime_session_state::state::SessionSetup;

mod tests;

// Per-connection caps for pre-setup audio frames: LiteLLM counts messages and UTF-8 bytes together
pub const MAX_BUFFERED_MESSAGES: usize = 200;
pub const MAX_BUFFERED_BYTES: usize = 10 * 1024 * 1024;

const APPEND_TYPE: &str = "input_audio_buffer.append";
const CLEAR_TYPE: &str = "input_audio_buffer.clear";
const CLIENT_AUDIO_BUFFER_TYPES: [&str; 4] = [
    APPEND_TYPE,
    "input_audio_buffer.commit",
    CLEAR_TYPE,
    "input_audio_buffer.end",
];
// The transport caps one frame before buffering; this only bounds the parser's own allocation
const FRAME_LIMITS: RealtimeDecodeLimits = RealtimeDecodeLimits { max_bytes: 1 << 24 };

#[derive(Debug, thiserror::Error)]
pub enum RealtimeSetupBufferError {
    #[error("the realtime frame is not a JSON object")]
    NotAnObject,
    #[error("the realtime frame cannot be decoded: {0}")]
    Decode(#[from] RealtimeContractError),
}

// None stands for every frame LiteLLM's json.loads refuses, which the callers treat as a non-audio frame
fn frame_type(message: &str) -> Result<Option<String>, RealtimeSetupBufferError> {
    let decoded = match decode_realtime_frame(message.as_bytes(), FRAME_LIMITS) {
        Ok(decoded) => decoded,
        Err(RealtimeContractError::Json(_source)) => return Ok(None),
        Err(error) => return Err(RealtimeSetupBufferError::Decode(error)),
    };
    let Json::Object(fields) = decoded else {
        return Err(RealtimeSetupBufferError::NotAnObject);
    };
    match fields.get("type") {
        Some(Json::Text(kind)) => Ok(Some(kind.clone())),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => Ok(None),
    }
}

// A clear drops the appends after the last commit and is never replayed itself
fn collapse_audio_messages(messages: &[String]) -> Result<Vec<String>, RealtimeSetupBufferError> {
    let mut collapsed = Vec::new();
    let mut pending_appends = Vec::new();
    for message in messages {
        match frame_type(message)?.as_deref() {
            Some(APPEND_TYPE) => pending_appends.push(message.clone()),
            Some(CLEAR_TYPE) => pending_appends.clear(),
            _ => {
                collapsed.append(&mut pending_appends);
                collapsed.push(message.clone());
            }
        }
    }
    collapsed.append(&mut pending_appends);
    Ok(collapsed)
}

// The failed batch goes back in front of frames that arrived while it was sent, and the byte total follows the merge
fn restore_unsent_messages(setup: &mut SessionSetup, unsent: &[String]) {
    let mut retained = unsent.to_vec();
    retained.append(&mut setup.pending_messages_until_setup);
    setup.pending_messages_byte_total = retained.iter().map(String::len).sum();
    setup.pending_messages_until_setup = retained;
}

// The workspace denies a second inherent impl for these types, so each buffering method lives on a trait the callers import
pub trait SetupBuffer {
    fn uses_deferred_backend_setup(&self) -> bool;

    fn should_buffer_until_setup(&self, message: &str) -> Result<bool, RealtimeSetupBufferError>;

    fn buffer_pending_message(&mut self, message: &str) -> Result<(), RealtimeSetupBufferError>;
}

#[async_trait]
pub trait SetupFlush {
    async fn flush_pending_messages(&mut self) -> Result<bool, RealtimeSetupBufferError>;
}

impl SetupBuffer for RealtimeStreaming {
    // A provider that needs no session configuration has setup complete from the first frame
    fn uses_deferred_backend_setup(&self) -> bool {
        self.provider_config
            .as_deref()
            .is_some_and(|config| !config.requires_session_configuration())
    }

    fn should_buffer_until_setup(&self, message: &str) -> Result<bool, RealtimeSetupBufferError> {
        if !self.uses_deferred_backend_setup() {
            return Ok(false);
        }
        if self.setup.backend_setup_complete.0 && !self.setup.flushing_pending_messages.0 {
            return Ok(false);
        }
        Ok(frame_type(message)?
            .is_some_and(|kind| CLIENT_AUDIO_BUFFER_TYPES.contains(&kind.as_str())))
    }

    fn buffer_pending_message(&mut self, message: &str) -> Result<(), RealtimeSetupBufferError> {
        let kind = frame_type(message)?;
        if kind.as_deref() == Some(CLEAR_TYPE) {
            let mut messages = self.setup.pending_messages_until_setup.clone();
            messages.push(message.to_owned());
            let collapsed = collapse_audio_messages(&messages)?;
            self.setup.pending_messages_byte_total = collapsed.iter().map(String::len).sum();
            self.setup.pending_messages_until_setup = collapsed;
            return Ok(());
        }
        let size = message.len();
        let total = self.setup.pending_messages_byte_total.saturating_add(size);
        if self.setup.pending_messages_until_setup.len() < MAX_BUFFERED_MESSAGES
            && total <= MAX_BUFFERED_BYTES
        {
            self.setup
                .pending_messages_until_setup
                .push(message.to_owned());
            self.setup.pending_messages_byte_total = total;
        } else {
            tracing::warn!(
                "Pre-setup buffer full ({} messages / {} bytes); dropping frame",
                self.setup.pending_messages_until_setup.len(),
                self.setup.pending_messages_byte_total
            );
        }
        Ok(())
    }
}

#[async_trait]
impl<Config, CallbackError> SetupFlush for RealtimeSender<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    // The batch leaves the queue before the first await, so a frame that arrives meanwhile stays queued and a failed batch is restored in front of it
    async fn flush_pending_messages(&mut self) -> Result<bool, RealtimeSetupBufferError> {
        let pending = collapse_audio_messages(&self.state.setup.pending_messages_until_setup)?;
        self.state.setup.pending_messages_until_setup.clear();
        self.state.setup.pending_messages_byte_total = 0;
        for (index, message) in pending.iter().enumerate() {
            let Err(error) = self.send_to_backend(message).await else {
                continue;
            };
            let unsent = pending.get(index..).unwrap_or_default();
            restore_unsent_messages(&mut self.state.setup, unsent);
            tracing::debug!(
                "Failed to flush buffered client message after setup: {error} ({} buffered message(s) retained)",
                unsent.len()
            );
            return Ok(false);
        }
        Ok(true)
    }
}
