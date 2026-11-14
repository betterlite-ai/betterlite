pub mod config;
pub mod prompt;
pub mod response;
pub mod schema;
pub mod tool_calls;
pub mod wire;

mod fields;
mod tests;

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 2] =
    [GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall];
