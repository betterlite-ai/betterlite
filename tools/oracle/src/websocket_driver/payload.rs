use super::DriverError;
use crate::websocket_oracle::Frame;
use tokio_tungstenite::tungstenite::Message;

impl TryFrom<(Message, &Frame)> for Frame {
    type Error = DriverError;
    fn try_from((message, expected): (Message, &Frame)) -> Result<Self, Self::Error> {
        let (text, binary, binary_data) = match message {
            Message::Text(text) => (Some(text), None, None),
            Message::Binary(bytes) => (None, expected.binary.clone(), Some(bytes)),
            Message::Close(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {
                return Err(DriverError::Transport("expected a data frame".to_owned()));
            }
        };
        Ok(Frame {
            direction: expected.direction,
            action: expected.action,
            text,
            binary,
            binary_data,
            barrier: expected.barrier.clone(),
        })
    }
}
