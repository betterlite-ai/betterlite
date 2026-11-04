use super::DriverError;
use crate::websocket_oracle::Frame;
use tokio_tungstenite::tungstenite::Message;

impl TryFrom<&Frame> for Message {
    type Error = DriverError;
    fn try_from(frame: &Frame) -> Result<Self, Self::Error> {
        match &frame.text {
            Some(text) => Ok(Self::Text(text.clone())),
            None => frame
                .binary_data
                .clone()
                .map(Self::Binary)
                .ok_or_else(|| DriverError::Transport("binary fixture is missing".to_owned())),
        }
    }
}
