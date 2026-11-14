use thiserror::Error;

use crate::api::api_boundaries::LimitErrorFields;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("Messages stream exceeded its retained-output limit")]
pub struct RetainedOutputLimit;

impl RetainedOutputLimit {
    pub const FIELDS: LimitErrorFields = LimitErrorFields {
        status: 502,
        error_type: "bad_gateway",
        param: "stream",
        code: "stream_buffer_too_large",
    };
}

// B1087's byte and item limits bound the encoded output kept for the billing call
pub struct RetentionBudget {
    pub max_bytes: usize,
    pub max_chunks: usize,
    pub bytes: usize,
    pub chunks: usize,
}

impl RetentionBudget {
    pub fn accept(&mut self, chunk_bytes: usize) -> Result<(), RetainedOutputLimit> {
        let bytes = self
            .bytes
            .checked_add(chunk_bytes)
            .ok_or(RetainedOutputLimit)?;
        let chunks = self.chunks.checked_add(1).ok_or(RetainedOutputLimit)?;
        if bytes > self.max_bytes || chunks > self.max_chunks {
            return Err(RetainedOutputLimit);
        }
        self.bytes = bytes;
        self.chunks = chunks;
        Ok(())
    }
}
