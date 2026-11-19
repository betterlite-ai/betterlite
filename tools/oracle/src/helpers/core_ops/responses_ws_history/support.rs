use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::responses_ws_history::error::FrameSinkError;
use betterlite_core::api::responses_ws_history::provider::{
    ProviderResolution, ProviderResolutionFailure,
};
use betterlite_core::api::responses_ws_history::serialization::serialize_chunk;
use betterlite_core::api::responses_ws_history::{
    Connection, FrameSink, ManagedResponsesWebSocket, WsDeps,
};
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_oracle_helper_api::error::HelperError;
use secrecy::SecretString;
use serde_json::Value;

use super::Setup;

const WARMUP_ID_HEX: &str = "0123456789abcdef0123456789abcdef";
const CLOCK_SECONDS: i64 = 1_765_000_000;

#[derive(Clone)]
pub(super) struct Socket {
    pub(super) frames: Arc<Mutex<Vec<String>>>,
    pub(super) fail_next: Arc<AtomicBool>,
}

#[async_trait]
impl FrameSink for Socket {
    async fn send_text(&self, frame: &str) -> Result<(), FrameSinkError> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Err(FrameSinkError {
                message: "the case asks the socket to refuse the frame".to_owned(),
            });
        }
        self.frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(frame.to_owned());
        Ok(())
    }
}

pub(super) fn frame_count(socket: &Socket) -> usize {
    socket
        .frames
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len()
}

pub(super) fn sent_frames(socket: &Socket, start: usize) -> Result<Vec<Value>, Box<HelperError>> {
    let frames = socket.frames.lock().unwrap_or_else(PoisonError::into_inner);
    let mut parsed = Vec::new();
    for frame in frames.get(start..).unwrap_or(&[]) {
        let value = serde_json::from_str::<Value>(frame)
            .map_err(|error| helper_error("ValueError", None, error.to_string()))?;
        parsed.push(value);
    }
    Ok(parsed)
}

pub(super) struct ScriptedResolver;

impl ProviderResolution for ScriptedResolver {
    fn resolve(&self, model: &str) -> Result<String, ProviderResolutionFailure> {
        match model.split_once('/') {
            Some(("openai", _rest)) => Ok("openai".to_owned()),
            Some(("anthropic", _rest)) => Ok("anthropic".to_owned()),
            _ => Err(ProviderResolutionFailure {
                message: format!("no provider answers {model}"),
            }),
        }
    }
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

fn logging_object() -> Arc<StreamLoggingObject> {
    Arc::new(StreamLoggingObject {
        slot: Mutex::new(None),
        dispatch: Arc::new(NoDispatch),
    })
}

pub(super) fn handler(setup: &Setup, socket: Socket) -> ManagedResponsesWebSocket<Socket> {
    let connection = Connection {
        websocket: socket,
        model: setup.model.clone(),
        logging_obj: logging_object(),
        user_api_key_dict: None,
        litellm_metadata: setup.litellm_metadata.clone(),
        api_key: setup.api_key.clone().map(SecretString::from),
        api_base: setup.api_base.clone(),
        timeout: setup.timeout,
        custom_llm_provider: setup.custom_llm_provider.clone(),
        first_message: setup.first_message.clone(),
        quota_callbacks: None,
        kwargs: setup.kwargs.clone(),
    };
    ManagedResponsesWebSocket::new(
        connection,
        WsDeps {
            resolver: Arc::new(ScriptedResolver),
            clock: Arc::new(|| CLOCK_SECONDS),
            ids: Arc::new(|| WARMUP_ID_HEX.to_owned()),
            serializer: Arc::new(serialize_chunk),
        },
    )
}

pub(super) fn helper_error(class: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}
