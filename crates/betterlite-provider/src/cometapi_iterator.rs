use betterlite_openai::cometapi_wire::chunk::{CometApiChunk, chunk_parser};
use betterlite_openai::cometapi_wire::error::CometApiError;
use betterlite_prompts::arguments::json::Json;
use betterlite_streaming::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
use thiserror::Error;
mod tests;
const DONE_MARKER: &str = "[DONE]";
#[derive(Debug, Error)]
pub enum CometApiIteratorError {
    #[error(transparent)]
    Api(#[from] CometApiError),
    #[error("the CometAPI stream chunk is not a JSON object")]
    NotAnObject,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamStep {
    Chunk(Box<CometApiChunk>),
    Empty,
    Blank,
    Done,
}
#[derive(Debug, Default)]
pub struct CometApiChatIterator {
    done: bool,
}
impl CometApiChatIterator {
    // B010's line lifecycle: blank lines are skipped, [DONE] wins, and a stripped line parses to CometAPI's record
    pub fn next_line(&mut self, line: &str) -> Result<StreamStep, CometApiIteratorError> {
        if self.done {
            return Ok(StreamStep::Done);
        }
        if line.trim().is_empty() {
            return Ok(StreamStep::Blank);
        }
        if line.contains(DONE_MARKER) {
            self.done = true;
            return Ok(StreamStep::Done);
        }
        let payload = strip_sse_data_from_chunk(Some(line)).unwrap_or(line);
        let Ok(parsed) = serde_json::from_str::<Json>(payload) else {
            return Ok(StreamStep::Empty);
        };
        // Python's `elif stripped_json_chunk:` drops a parsed falsy value before chunk_parser sees it
        if !parsed.is_truthy() {
            return Ok(StreamStep::Empty);
        }
        let Json::Object(fields) = parsed else {
            return Err(CometApiIteratorError::NotAnObject);
        };
        Ok(StreamStep::Chunk(Box::new(chunk_parser(&fields)?)))
    }
    pub const fn close(&mut self) -> StreamStep {
        self.done = true;
        StreamStep::Done
    }
}
