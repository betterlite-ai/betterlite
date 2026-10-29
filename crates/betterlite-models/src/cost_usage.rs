pub mod error;
mod field;
pub mod interactions;
mod token_count;
pub mod transcription;

/// A billing projection a caller copies into its own `Usage` record. Not a wire type:
/// callers own how it gets serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CostUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub prompt_details: Option<PromptDetails>,
    pub completion_details: Option<CompletionDetails>,
    pub cache_read_tokens: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PromptDetails {
    pub text_tokens: Option<i64>,
    pub audio_tokens: Option<i64>,
    pub image_tokens: Option<i64>,
    pub video_tokens: Option<i64>,
    pub web_search_requests: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompletionDetails {
    pub text_tokens: Option<i64>,
    pub audio_tokens: Option<i64>,
    pub image_tokens: Option<i64>,
    pub video_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
}

mod tests;
