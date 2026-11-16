pub mod any_value;
pub mod attributes;
pub mod error;
pub mod events;
pub mod filter;
pub mod inference_details;
pub mod latency;
pub mod legacy_content_events;
pub mod recorder;

pub const PROMPT_EVENT: &str = "gen_ai.content.prompt";
pub const COMPLETION_EVENT: &str = "gen_ai.content.completion";
pub const INFERENCE_DETAILS_EVENT: &str = "gen_ai.client.inference.operation.details";

// LiteLLM's fn _build_inference_details_attrs and fn _emit_inference_details_event are inference_details::build_attributes and emit_inference_details
