use std::collections::VecDeque;

use value::Json;

mod frames;
mod python_json;
mod tests;
pub mod value;

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
        let mut frames = vec![frames::frame(
            "message_start",
            &frames::message_start(response),
        )];
        for (index, block) in frames::content_items(response)?.iter().enumerate() {
            frames::content_block_frames(&mut frames, block, index)?;
        }
        frames.push(frames::frame(
            "message_delta",
            &frames::message_delta(response),
        ));
        frames.push(frames::frame(
            "message_stop",
            &frames::message_stop(response),
        ));
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
