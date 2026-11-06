use betterlite_models::prompt_spec::{PromptLiteLLMParams, PromptSpec};

use crate::gitlab_client::transport::GitLabTransport;

use super::super::error::InitializerError;
use super::config;
use super::manager::GitLabPromptManager;

pub fn prompt_initializer<T>(
    litellm_params: &PromptLiteLLMParams,
    prompt_spec: &PromptSpec,
    transport: T,
) -> Result<GitLabPromptManager<T>, InitializerError>
where
    T: GitLabTransport,
{
    let gitlab_config = config::gitlab_config(litellm_params)?;
    let git_ref = config::git_ref(litellm_params)?;
    Ok(GitLabPromptManager::new(
        gitlab_config,
        Some(prompt_spec.prompt_id.clone()),
        git_ref,
        None,
        transport,
    )?)
}

mod tests;
