use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use futures_util::stream::{self, Stream, StreamExt as _};
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;

use super::error::AimError;
use super::input;
use super::settings::{AimGuardrail, AimHook, DEFAULT_TIMEOUT};

pub const MAX_STREAM_FRAME_BYTES: usize = 1024 * 1024;
// websockets' client waits ten seconds for the handshake unless the caller says otherwise
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DONE_FRAME: &str = r#"{"done":true}"#;

#[async_trait]
pub trait AimFrameReader: Send {
    async fn recv_text(&mut self) -> Result<Option<String>, AimError>;
}

#[async_trait]
pub trait AimFrameWriter: Send {
    async fn send_text(&mut self, text: String) -> Result<(), AimError>;
}

pub struct AimSocket {
    pub reader: Box<dyn AimFrameReader>,
    pub writer: Box<dyn AimFrameWriter>,
}

#[async_trait]
pub trait AimSocketConnector: Send + Sync {
    async fn connect(
        &self,
        url: &str,
        headers: &HeaderMap,
        open: Duration,
        read: Duration,
    ) -> Result<AimSocket, AimError>;
}

// one response chunk as the transport received it, serialized JSON the sender forwards to Aim
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AimStreamChunk(pub String);

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct VerifiedChunk {
    pub choices: Vec<VerifiedChoice>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct VerifiedChoice {
    pub index: Option<i64>,
    pub delta: VerifiedDelta,
    pub finish_reason: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct VerifiedDelta {
    pub content: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, GuardrailValue>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StreamFrame {
    verified_chunk: Option<VerifiedChunk>,
    done: Option<bool>,
    blocking_message: Option<String>,
}

impl AimGuardrail {
    pub async fn inspect_output_stream<S>(
        &self,
        chunks: S,
        request_data: &BTreeMap<String, GuardrailValue>,
        key_alias: Option<&str>,
    ) -> Result<impl Stream<Item = Result<VerifiedChunk, AimError>>, AimError>
    where
        S: Stream<Item = AimStreamChunk> + Send + 'static,
    {
        let headers = self.build_headers(
            AimHook::Output,
            key_alias,
            input::user_email(request_data),
            input::call_id(request_data),
        )?;
        let url = format!("{}/fw/v1/analyze/stream", self.ws_api_base);
        let socket = self
            .connector
            .connect(
                &url,
                &headers,
                CONNECT_TIMEOUT,
                self.timeout.unwrap_or(DEFAULT_TIMEOUT),
            )
            .await?;
        Ok(open_socket(socket, chunks))
    }
}

fn open_socket<S>(
    socket: AimSocket,
    chunks: S,
) -> impl Stream<Item = Result<VerifiedChunk, AimError>>
where
    S: Stream<Item = AimStreamChunk> + Send + 'static,
{
    let AimSocket { reader, mut writer } = socket;
    let mut sender = JoinSet::new();
    sender.spawn(async move { forward_chunks(writer.as_mut(), chunks).await });
    stream::unfold(
        StreamState {
            reader,
            sender: Some(sender),
            finished: false,
        },
        |mut state| async move { state.next_item().await.map(|item| (item, state)) },
    )
}

async fn forward_chunks<S>(writer: &mut dyn AimFrameWriter, chunks: S)
where
    S: Stream<Item = AimStreamChunk>,
{
    let mut chunks = chunks;
    while let Some(chunk) = chunks.next().await {
        if writer.send_text(chunk.0).await.is_err() {
            return;
        }
    }
    if writer.send_text(DONE_FRAME.to_owned()).await.is_err() {
        return;
    }
}

struct StreamState {
    reader: Box<dyn AimFrameReader>,
    sender: Option<JoinSet<()>>,
    finished: bool,
}

impl StreamState {
    async fn next_item(&mut self) -> Option<Result<VerifiedChunk, AimError>> {
        if self.finished {
            return None;
        }
        match self.reader.recv_text().await {
            Ok(Some(text)) => self.handle_text(&text),
            Ok(None) => {
                self.finish();
                Some(Err(AimError::StreamClosed))
            }
            Err(error) => {
                self.finish();
                Some(Err(error))
            }
        }
    }

    fn handle_text(&mut self, text: &str) -> Option<Result<VerifiedChunk, AimError>> {
        let frame = match serde_json::from_str::<StreamFrame>(text) {
            Ok(frame) => frame,
            Err(source) => {
                self.finish();
                return Some(Err(AimError::StreamJson(source)));
            }
        };
        if let Some(chunk) = frame.verified_chunk {
            return Some(Ok(chunk));
        }
        self.finish();
        if frame.done == Some(true) {
            return None;
        }
        frame
            .blocking_message
            .map(|message| Err(AimError::StreamBlocked { message }))
    }

    fn finish(&mut self) {
        self.finished = true;
        drop(self.sender.take());
    }
}
