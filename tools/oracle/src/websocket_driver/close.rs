use super::DriverError;
use crate::websocket_oracle::Close;
use tokio_tungstenite::tungstenite::Message;

impl TryFrom<(Message, &str)> for Close {
    type Error = DriverError;
    fn try_from((message, initiator): (Message, &str)) -> Result<Self, Self::Error> {
        let Message::Close(Some(frame)) = message else {
            return Err(DriverError::Transport("expected a close frame".to_owned()));
        };
        Ok(Close {
            initiator: initiator.to_owned(),
            code: u16::from(frame.code),
            reason: frame.reason.into_owned(),
        })
    }
}
