use std::error::Error as StdError;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_native_stream::retention::RetentionBudget;

use super::StreamEvent;
use super::error::PresidioStreamError;

// What a buffering hook holds is untrusted provider output, so the API boundary ceilings of 16 MiB and 64 * 1024 items bound it
pub struct Retention {
    pub budget: RetentionBudget,
}

impl Default for Retention {
    fn default() -> Self {
        let limits = ApiLimits::default();
        Self {
            budget: RetentionBudget {
                max_bytes: limits.max_json_bytes,
                max_chunks: limits.max_items,
                bytes: 0,
                chunks: 0,
            },
        }
    }
}

impl Retention {
    pub fn accept<Failure>(
        &mut self,
        chunk_bytes: usize,
    ) -> Result<(), PresidioStreamError<Failure>>
    where
        Failure: StdError + 'static,
    {
        self.budget
            .accept(chunk_bytes)
            .map_err(|source| PresidioStreamError::OverBudget {
                bytes: self.budget.max_bytes,
                chunks: self.budget.max_chunks,
                source,
            })
    }
}

// The bytes an event holds while it is buffered
#[must_use]
pub fn event_len(event: &StreamEvent) -> usize {
    match event {
        StreamEvent::Bytes(frame) => frame.len(),
        StreamEvent::Chat(chunk) | StreamEvent::Other(chunk) => chunk.get().len(),
    }
}
