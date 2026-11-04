use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use super::DriverError;

pub(super) struct Peer<S> {
    pub(super) stream: WebSocketStream<S>,
}

impl<S> Peer<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub(super) async fn receive(&mut self) -> Result<Message, DriverError> {
        loop {
            let message = self
                .stream
                .next()
                .await
                .ok_or_else(|| {
                    DriverError::Transport("socket ended before expected message".to_owned())
                })?
                .map_err(|error| DriverError::Transport(error.to_string()))?;
            match message {
                Message::Ping(payload) => self
                    .stream
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|error| DriverError::Transport(error.to_string()))?,
                Message::Pong(_) | Message::Frame(_) => {}
                Message::Text(_) | Message::Binary(_) | Message::Close(_) => return Ok(message),
            }
        }
    }
}
