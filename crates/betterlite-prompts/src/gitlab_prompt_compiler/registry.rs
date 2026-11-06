use std::future::Future;
use std::pin::Pin;

use betterlite_models::prompt_spec::{
    PromptLiteLLMParams, PromptSpec, SupportedPromptIntegrations,
};

use crate::gitlab_client::transport::GitLabTransport;

use super::error::InitializerError;
use super::init::factory::prompt_initializer;
use super::init::manager::GitLabPromptManager;

pub type InitializerFn<T> = fn(
    &PromptLiteLLMParams,
    &PromptSpec,
    T,
) -> Pin<
    Box<dyn Future<Output = Result<GitLabPromptManager<T>, InitializerError>> + Send>,
>;

// Every variant is matched explicitly so a new integration fails to compile until wired up.
#[must_use]
pub fn prompt_initializer_registry<T>(
    integration: &SupportedPromptIntegrations,
) -> Option<InitializerFn<T>>
where
    T: GitLabTransport + 'static,
{
    match integration {
        SupportedPromptIntegrations::Gitlab => Some(prompt_initializer::<T>),
        SupportedPromptIntegrations::DotPrompt
        | SupportedPromptIntegrations::Langfuse
        | SupportedPromptIntegrations::Custom
        | SupportedPromptIntegrations::Bitbucket
        | SupportedPromptIntegrations::GenericPromptManagement
        | SupportedPromptIntegrations::ArizePhoenix => None,
    }
}

mod tests;
