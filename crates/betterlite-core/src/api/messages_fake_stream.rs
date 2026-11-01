use std::collections::VecDeque;

use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;

mod frames;
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MessagesFakeStreamError {
    #[error("'NoneType' object is not iterable")]
    ContentNotIterable,
    #[error("'NoneType' object has no attribute 'get'")]
    ContentBlockNotMapping,
}

pub struct MessagesFakeStream {
    frames: VecDeque<Vec<u8>>,
}

impl MessagesFakeStream {
    pub fn new(response: &Json) -> Result<Self, MessagesFakeStreamError> {
        let mut frames = vec![
            format!(
                "event: message_start\ndata: {}\n\n",
                dumps(&frames::message_start(response))
            )
            .into_bytes(),
        ];
        for (index, block) in frames::content_items(response)?.iter().enumerate() {
            frames::content_block_frames(&mut frames, block, index)?;
        }
        frames.push(
            format!(
                "event: message_delta\ndata: {}\n\n",
                dumps(&frames::message_delta(response))
            )
            .into_bytes(),
        );
        frames.push(
            format!(
                "event: message_stop\ndata: {}\n\n",
                dumps(&frames::message_stop(response))
            )
            .into_bytes(),
        );
        Ok(Self {
            frames: VecDeque::from(frames),
        })
    }
}

impl Iterator for MessagesFakeStream {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        self.frames.pop_front()
    }
}
