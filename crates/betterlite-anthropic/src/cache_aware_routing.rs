pub mod deployment;
pub mod predict_arm;
pub mod services;
pub mod types;

mod cost;
mod count_tokens;
mod fits;
mod from_body;
mod is_warm;
mod minimum_tokens;
mod predict;
mod request_body;
mod supports;
pub(crate) mod tests;
mod validation;

use betterlite_models::decisions::Json;

use crate::cache_plan::PromptPrefix;

// LiteLLM's frozen dataclass: the predicted body, the prefix parsed from it and the caller's output limit.
#[derive(Clone, Debug)]
pub struct AnthropicCacheRouting {
    pub body: Json,
    pub prefix: PromptPrefix,
    pub requested_output_limit: i64,
}
