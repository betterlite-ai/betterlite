use super::DriverError;
use crate::websocket_oracle::Frame;
use tokio_tungstenite::tungstenite::Message;

impl TryFrom<&Frame> for Message {
    type Error = DriverError;
    fn try_from(frame: &Frame) -> Result<Self, Self::Error> {
        frame.text.as_ref().map_or_else(
            || {
                frame
                    .binary_data
                    .as_ref()
                    .map(|bytes| Self::binary(bytes.clone()))
                    .ok_or_else(|| DriverError::Transport("binary fixture is missing".to_owned()))
            },
            |text| Ok(Self::text(text.clone())),
        )
    }
}
