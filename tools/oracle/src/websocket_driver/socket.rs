use futures_util::{SinkExt as _, StreamExt as _};
use std::collections::VecDeque;
use std::ptr;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{
    Message, Utf8Bytes,
    protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode},
};

use super::DriverError;
use crate::websocket_oracle::{
    Close, Direction, Frame, MAX_CAPTURE_BYTES, MAX_FRAME_BYTES, MAX_FRAMES, WebSocketCase,
};

pub(super) struct Peer<S> {
    pub(super) stream: WebSocketStream<S>,
    pub(super) bytes: usize,
    pub(super) count: usize,
}

pub(super) fn config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME_BYTES))
        .max_frame_size(Some(MAX_FRAME_BYTES))
        .max_write_buffer_size(0x0020_0000)
}

impl<S: AsyncRead + AsyncWrite + Unpin> Peer<S> {
    fn account(&mut self, message: &Message) -> Result<(), DriverError> {
        self.bytes = self.bytes.saturating_add(message.len());
        if message.is_text() || message.is_binary() {
            self.count = self.count.saturating_add(1);
        }
        if self.bytes > MAX_CAPTURE_BYTES || self.count > MAX_FRAMES {
            return Err(DriverError::Transport(
                "captured traffic exceeds limit".to_owned(),
            ));
        }
        Ok(())
    }

    async fn receive(&mut self) -> Result<Message, DriverError> {
        loop {
            let message = self.stream.next().await.ok_or_else(|| {
                DriverError::Transport("socket ended before expected message".to_owned())
            })??;
            self.account(&message)?;
            match message {
                Message::Ping(payload) => self.stream.send(Message::Pong(payload)).await?,
                Message::Pong(_) | Message::Frame(_) => {}
                Message::Text(_) | Message::Binary(_) | Message::Close(_) => return Ok(message),
            }
        }
    }

    async fn receive_until(
        &mut self,
        pending: &mut VecDeque<&Frame>,
        observed: &mut Vec<Frame>,
        barrier: Option<&str>,
    ) -> Result<(), DriverError> {
        while let Some(expected) = pending.pop_front() {
            observed.push(Frame::try_from((self.receive().await?, expected))?);
            if barrier.is_none() || expected.barrier.as_deref() == barrier {
                return Ok(());
            }
        }
        Err(DriverError::Transport(
            "unresolved barrier or exhausted script".to_owned(),
        ))
    }

    pub(super) async fn play(
        &mut self,
        case: &WebSocketCase,
        sending: Direction,
    ) -> Result<(Vec<Frame>, Option<Close>), DriverError> {
        let mut pending: VecDeque<_> = case
            .frames
            .iter()
            .filter(|frame| frame.direction != sending)
            .collect();
        let mut observations = Vec::new();
        for frame in &case.frames {
            if frame.direction == sending {
                self.send_frame(frame, &mut pending, &mut observations)
                    .await?;
            } else if pending.front().is_some_and(|next| ptr::eq(*next, frame)) {
                self.receive_until(&mut pending, &mut observations, None)
                    .await?;
            } else {
                // A barrier already consumed this observation.
            }
        }
        let close = self.finish_close(case.close.as_ref(), sending).await?;
        Ok((observations, close))
    }

    async fn send_frame(
        &mut self,
        frame: &Frame,
        pending: &mut VecDeque<&Frame>,
        observed: &mut Vec<Frame>,
    ) -> Result<(), DriverError> {
        if frame.direction == Direction::ClientToProvider
            && let Some(barrier) = frame.barrier.as_deref()
            && !observed.iter().any(|seen| {
                seen.direction == Direction::ProviderToClient
                    && seen.barrier.as_deref() == Some(barrier)
            })
        {
            self.receive_until(pending, observed, Some(barrier)).await?;
        }
        let message = Message::try_from(frame)?;
        self.account(&message)?;
        self.stream.send(message).await?;
        observed.push(frame.clone());
        Ok(())
    }

    async fn finish_close(
        &mut self,
        expected: Option<&Close>,
        sending: Direction,
    ) -> Result<Option<Close>, DriverError> {
        let Some(close) = expected else {
            return Ok(None);
        };
        let local = match sending {
            Direction::ClientToProvider => "client",
            Direction::ProviderToClient => "provider",
        };
        if close.initiator == local {
            let message = CloseFrame {
                code: CloseCode::from(close.code),
                reason: Utf8Bytes::from(close.reason.clone()),
            };
            self.stream.send(Message::Close(Some(message))).await?;
            Ok(Some(close.clone()))
        } else {
            Ok(Some(Close::try_from((
                self.receive().await?,
                close.initiator.as_str(),
            ))?))
        }
    }
}
