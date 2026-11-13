pub const CLEAR_TOOL_USES_EDIT_TYPE: &str = "clear_tool_uses_20250919";
pub const DEFAULT_INPUT_TOKENS_TRIGGER: u64 = 100_000;
pub const DEFAULT_KEEP_TOOL_USES: u64 = 3;
pub const CLEARED_TOOL_RESULT_CONTENT: &str = "[Cleared by context management]";
pub const COMPACT_EDIT_TYPE: &str = "compact_20260112";
pub const COMPACT_DEFAULT_TRIGGER_TOKENS: u64 = 150_000;
pub const COMPACT_MIN_TRIGGER_TOKENS: u64 = 50_000;
pub const COMPACT_SUMMARY_MAX_TOKENS: u64 = 4096;
pub const SUMMARY_MAX_TOKENS_KEY: &str = "context_management_summary_max_tokens";
pub const SUMMARY_TIMEOUT_SECONDS: f64 = 60.0;
pub const SUMMARY_MODEL_KEY: &str = "context_management_summary_model";
pub const COMPACT_SUMMARY_SYSTEM_PREFIX: &str = "Previous conversation summary: ";
pub const COMPACT_DEFAULT_INSTRUCTIONS: &str = concat!(
    "You have written a partial transcript for the initial task above. Please ",
    "write a summary of the transcript. The purpose of this summary is to ",
    "provide continuity so you can continue to make progress towards solving ",
    "the task in a future context, where the raw history above may not be ",
    "accessible and will be replaced with this summary. Write down anything that ",
    "would be helpful, including the state, next steps, learnings etc. ",
    "You must wrap your summary in a <summary></summary> block."
);
pub const COMPACT_NO_TOOL_CALLS_SUFFIX: &str =
    " Do not call any tools while writing this summary; respond with text only.";
