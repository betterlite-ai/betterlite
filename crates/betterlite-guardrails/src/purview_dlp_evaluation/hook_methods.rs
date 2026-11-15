use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

// LiteLLM's get_config_model reports that no UI config model is built yet; None is the whole value.
pub const CONFIG_MODEL: Option<GuardrailValue> = None;

#[must_use]
pub const fn get_supported_event_hooks() -> [GuardrailEventHooks; 3] {
    [
        GuardrailEventHooks::PreCall,
        GuardrailEventHooks::PostCall,
        GuardrailEventHooks::LoggingOnly,
    ]
}

mod tests;
