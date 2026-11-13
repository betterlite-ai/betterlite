use super::DriverError;
use crate::websocket_oracle::{Close, Initiator};
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
