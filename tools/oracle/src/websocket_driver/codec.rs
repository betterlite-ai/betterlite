use super::DriverError;
use crate::websocket_oracle::{Close, Frame, Initiator};
use tokio_tungstenite::tungstenite::Message;

impl TryFrom<(Message, Initiator)> for Close {
    type Error = DriverError;
    fn try_from((message, initiator): (Message, Initiator)) -> Result<Self, Self::Error> {
        let Message::Close(Some(frame)) = message else {
            return Err(DriverError::Transport("expected a close frame".to_owned()));
        };
        Ok(Self {
            initiator,
            code: u16::from(frame.code),
            reason: frame.reason.to_string(),
        })
    }
}

impl TryFrom<&Frame> for Message {
    type Error = DriverError;
    fn try_from(frame: &Frame) -> Result<Self, Self::Error> {
        match (&frame.text, &frame.binary_data) {
            (Some(text), _) => Ok(Self::text(text.clone())),
            (None, Some(bytes)) => Ok(Self::binary(bytes.clone())),
            (None, None) => Err(DriverError::Transport(
                "binary fixture is missing".to_owned(),
            )),
        }
    }
}

impl TryFrom<(Message, &Self)> for Frame {
    type Error = DriverError;
    fn try_from((message, expected): (Message, &Self)) -> Result<Self, Self::Error> {
        let (text, binary, binary_data) = match message {
            Message::Text(text) => (Some(text.to_string()), None, None),
            Message::Binary(bytes) => (None, expected.binary.clone(), Some(bytes.to_vec())),
            Message::Close(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {
                return Err(DriverError::Transport("expected a data frame".to_owned()));
            }
        };
        Ok(Self {
            direction: expected.direction,
            action: expected.action,
            text,
            binary,
            binary_data,
            barrier: expected.barrier.clone(),
        })
    }
}
