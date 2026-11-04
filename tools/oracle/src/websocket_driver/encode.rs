use super::DriverError;
use crate::websocket_oracle::Frame;
use tokio_tungstenite::tungstenite::Message;

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
