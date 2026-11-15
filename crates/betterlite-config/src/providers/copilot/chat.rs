use crate::loader::EnvironmentLookup;
use crate::sections::copilot_chat::Settings;

const API_BASE_VAR: &str = "GITHUB_COPILOT_API_BASE";

pub(crate) fn resolve(
    lookup: &dyn EnvironmentLookup,
    disable_system_to_assistant: Option<bool>,
) -> Settings {
    Settings {
        disable_system_to_assistant: disable_system_to_assistant.unwrap_or(false),
        // LiteLLM reads the variable behind an `or` chain, so an empty value falls through to the default base
        api_base: lookup
            .lookup(API_BASE_VAR)
            .filter(|value| !value.is_empty()),
    }
}
