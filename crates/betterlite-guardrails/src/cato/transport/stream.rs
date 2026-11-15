use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::header::HeaderMap;
use tokio::sync::mpsc;
use tokio::task::{JoinError, JoinSet};

use crate::cato::CatoGuardrail;
use crate::cato::error::{CatoError, ModelStreamError};
use crate::cato::wire::StreamMessage;

// websockets' client caps a message at one mebibyte unless the caller says otherwise
pub(crate) const MAX_FRAME_BYTES: usize = 1024 * 1024;
// websockets' client waits ten seconds for the handshake unless the caller says otherwise
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
// LiteLLM sends json.dumps({"done": True}) when the model stream ends
pub(crate) const DONE_FRAME: &str = r#"{"done": true}"#;

#[async_trait]
pub(crate) trait CatoFrameReader: Send {
    // Ok(Some) is one upstream frame; Ok(None) means the upstream closed the connection
    async fn recv_message(&mut self) -> Result<Option<Vec<u8>>, CatoError>;
}

#[async_trait]
pub(crate) trait CatoFrameWriter: Send {
    async fn send_text(&mut self, text: &str) -> Result<(), CatoError>;
}

pub(crate) struct CatoSocket {
    pub(crate) reader: Box<dyn CatoFrameReader>,
    pub(crate) writer: Box<dyn CatoFrameWriter>,
}

#[async_trait]
pub(crate) trait CatoSocketConnector: Send + Sync {
    async fn connect(
        &self,
        url: &str,
        headers: &HeaderMap,
        open: Duration,
        read: Duration,
    ) -> Result<CatoSocket, CatoError>;
}

// LiteLLM's async_post_call_streaming_iterator_hook: chunks go upstream while the verified ones come back
pub(crate) async fn relay_output_stream(
    connector: &dyn CatoSocketConnector,
    guardrail: &CatoGuardrail,
    headers: HeaderMap,
    chunks: mpsc::Receiver<Result<GuardrailValue, ModelStreamError>>,
    output: &mpsc::Sender<GuardrailValue>,
) -> Result<(), CatoError> {
    let url = format!("{}/fw/v1/analyze/stream", guardrail.ws_api_base);
    let socket = connector
        .connect(&url, &headers, CONNECT_TIMEOUT, guardrail.timeout)
        .await?;
    let mut sender = JoinSet::new();
    sender.spawn(forward_chunks(socket.writer, chunks));
    let mut state = StreamState {
        reader: socket.reader,
        sender: Some(sender),
    };
    while let Some(chunk) = state.next_chunk().await? {
        if output.send(chunk).await.is_err() {
            // the consumer stopped reading, so cancelling the upstream forward is enough
            break;
        }
    }
    Ok(())
}

async fn forward_chunks(
    mut writer: Box<dyn CatoFrameWriter>,
    mut chunks: mpsc::Receiver<Result<GuardrailValue, ModelStreamError>>,
) -> Result<(), CatoError> {
    while let Some(chunk) = chunks.recv().await {
        // a failed model stream kills the sender before the done frame, as an exception ends LiteLLM's task
        let chunk = chunk.map_err(|source| CatoError::ModelStream { source })?;
        // LiteLLM sends a string chunk as it is and every other chunk as JSON
        let text = match &chunk {
            GuardrailValue::Text(text) => text.clone(),
            _ => serde_json::to_string(&chunk)
                .map_err(|source| CatoError::StreamChunkJson { source })?,
        };
        writer.send_text(&text).await?;
    }
    writer.send_text(DONE_FRAME).await
}

enum CatoFrame {
    Chunk(BTreeMap<String, GuardrailValue>),
    Done,
    Blocked(String),
    Unknown(StreamMessage),
}

// LiteLLM reads verified_chunk, then done, then blocking_message, and logs anything else
fn classify(mut frame: StreamMessage) -> CatoFrame {
    if let Some(chunk) = frame.verified_chunk.take() {
        return CatoFrame::Chunk(chunk);
    }
    if frame.done == Some(true) {
        return CatoFrame::Done;
    }
    match frame.blocking_message.take() {
        Some(message) => CatoFrame::Blocked(message),
        None => CatoFrame::Unknown(frame),
    }
}

enum Await {
    Sender(Option<Result<Result<(), CatoError>, JoinError>>),
    Message(Result<Option<Vec<u8>>, CatoError>),
}

struct StreamState {
    reader: Box<dyn CatoFrameReader>,
    // dropping the set cancels the forwarding task, as LiteLLM's finally block cancels its task
    sender: Option<JoinSet<Result<(), CatoError>>>,
}

impl StreamState {
    async fn next_chunk(&mut self) -> Result<Option<BTreeMap<String, GuardrailValue>>, CatoError> {
        let Some(bytes) = self.next_message().await? else {
            return Err(CatoError::StreamClosed);
        };
        let frame = serde_json::from_slice::<StreamMessage>(&bytes)
            .map_err(|source| CatoError::StreamJson { source })?;
        match classify(frame) {
            CatoFrame::Chunk(chunk) => Ok(Some(chunk)),
            CatoFrame::Done => Ok(None),
            CatoFrame::Blocked(message) => Err(CatoError::StreamBlocked { message }),
            CatoFrame::Unknown(frame) => {
                tracing::error!(?frame, "Cato: unknown message received");
                Ok(None)
            }
        }
    }

    // LiteLLM's _await_cato_message: a dead sender is reported before a waiting receive
    async fn next_message(&mut self) -> Result<Option<Vec<u8>>, CatoError> {
        loop {
            let Some(sender) = self.sender.as_mut() else {
                return self.reader.recv_message().await;
            };
            let step = tokio::select! {
                biased;
                joined = sender.join_next() => Await::Sender(joined),
                message = self.reader.recv_message() => Await::Message(message),
            };
            match step {
                // a sender that finished cleanly leaves the answer to the upstream messages
                Await::Sender(None | Some(Ok(Ok(())))) => self.sender = None,
                Await::Sender(Some(Ok(Err(error)))) => {
                    return Err(CatoError::StreamSenderFailed {
                        source: Box::new(error),
                    });
                }
                Await::Sender(Some(Err(source))) => return Err(CatoError::StreamTask { source }),
                Await::Message(message) => return message,
            }
        }
    }
}
