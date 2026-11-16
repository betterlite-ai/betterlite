use betterlite_core::api::api_boundaries::ApiLimits;

// LiteLLM buffers a streamed response with no bound at all; a public gateway needs a per-request ceiling on untrusted input.
pub const DEFAULT_MAX_STREAM_CHUNKS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamBufferLimits {
    pub max_chunks: usize,
    pub max_bytes: usize,
}

impl Default for StreamBufferLimits {
    fn default() -> Self {
        Self {
            max_chunks: DEFAULT_MAX_STREAM_CHUNKS,
            max_bytes: ApiLimits::default().max_json_bytes,
        }
    }
}

mod tests;
