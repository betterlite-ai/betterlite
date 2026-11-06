use std::future::Future;
use std::pin::Pin;

use betterlite_models::prompt_spec::{PromptLiteLLMParams, PromptSpec};

use crate::gitlab_client::transport::GitLabTransport;

use super::super::error::InitializerError;
use super::config;
use super::manager::GitLabPromptManager;

pub fn prompt_initializer<T>(
    litellm_params: &PromptLiteLLMParams,
    prompt_spec: &PromptSpec,
    transport: T,
) -> Pin<Box<dyn Future<Output = Result<GitLabPromptManager<T>, InitializerError>> + Send>>
where
    T: GitLabTransport + 'static,
{
    let prepared = config::gitlab_config(litellm_params).and_then(|gitlab_config| {
        config::git_ref(litellm_params).map(|git_ref| (gitlab_config, git_ref))
    });
    let prompt_id = Some(prompt_spec.prompt_id.clone());
    Box::pin(async move {
        let (gitlab_config, git_ref) = prepared?;
        Ok(GitLabPromptManager::new(gitlab_config, prompt_id, git_ref, None, transport).await?)
    })
}

mod tests;
