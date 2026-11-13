pub mod accumulate_string_content;
pub mod apply_error;
pub mod apply_input_texts;
pub mod apply_input_tool_calls;
pub mod apply_output_texts;
pub mod apply_output_tool_calls;
pub mod combine_texts;
pub mod convert_tool_call;
pub mod delta_tool_calls;
pub mod error_items;
pub mod extract_inputs;
pub mod extract_output_content;
pub mod extract_tool_names;
pub mod extracted_content;
pub mod first_choice_finished;
pub mod has_text_content;
pub mod json_access;
pub mod narrowed_to_choice;
pub mod not_run_reason;
pub mod streaming_ended;
pub mod text_slot_write;
pub mod tool_fingerprints;

mod fixtures;

// The block framing tests sit here because the task's checks select them by this module path.
mod block_sse_chunks;
mod block_usage;
mod blocked_identity;
mod sse_chunk;
mod stream_choices;
