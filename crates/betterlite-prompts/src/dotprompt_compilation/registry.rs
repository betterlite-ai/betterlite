use betterlite_models::prompt_spec::{
    PromptLiteLLMParams, PromptSpec, SupportedPromptIntegrations,
};

use super::global::GlobalPromptDirectory;
use super::init::factory::{DotpromptInitializerError, prompt_initializer};
use super::init::manager::DotpromptManager;

pub type InitializerFn = fn(
    &PromptLiteLLMParams,
    &PromptSpec,
    &GlobalPromptDirectory,
) -> Result<DotpromptManager, DotpromptInitializerError>;

// Every variant is matched explicitly so a new integration fails to compile until wired up.
#[must_use]
pub fn prompt_initializer_registry(
    integration: &SupportedPromptIntegrations,
) -> Option<InitializerFn> {
    match integration {
        SupportedPromptIntegrations::DotPrompt => Some(prompt_initializer),
        SupportedPromptIntegrations::Langfuse
        | SupportedPromptIntegrations::Custom
        | SupportedPromptIntegrations::Bitbucket
        | SupportedPromptIntegrations::Gitlab
        | SupportedPromptIntegrations::GenericPromptManagement
        | SupportedPromptIntegrations::ArizePhoenix => None,
    }
}

mod tests;
