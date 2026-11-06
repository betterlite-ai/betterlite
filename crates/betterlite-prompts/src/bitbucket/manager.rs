use betterlite_http::proxy::ProxiedClient;
use betterlite_models::prompt_contracts::PromptManagementClient;
use tokio::sync::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard, OnceCell};

use crate::arguments::json::{Json, Object};
use crate::bitbucket_client::config::{AuthMethod, BitBucketConfig};
use crate::bitbucket_prompt_manager::BitBucketTemplateManager;

use super::error::{BitBucketFailure, BitBucketPromptError};
use super::messages::parse_prompt_to_messages;
use super::params::{project_optional_params, template_metadata};

#[derive(Debug)]
pub struct BitBucketPromptManager {
    pub bitbucket_config: BitBucketConfig,
    pub prompt_id: Option<String>,
    pub transport: ProxiedClient,
    pub template_manager: OnceCell<AsyncMutex<BitBucketTemplateManager>>,
}

impl BitBucketPromptManager {
    pub const INTEGRATION_NAME: &'static str = "bitbucket";

    // LiteLLM builds the template manager on first use, and that build fetches the configured prompt.
    pub async fn prompt_manager(
        &self,
    ) -> Result<AsyncMutexGuard<'_, BitBucketTemplateManager>, BitBucketFailure> {
        let cell = if let Some(cell) = self.template_manager.get() {
            cell
        } else {
            let config = rebuilt_config(&self.bitbucket_config);
            let transport = self.transport.clone();
            let prompt_id = self.prompt_id.clone();
            self.template_manager
                .get_or_try_init(|| async move {
                    BitBucketTemplateManager::new(config, transport, prompt_id.as_deref())
                        .await
                        .map(AsyncMutex::new)
                        .map_err(BitBucketFailure::Manager)
                })
                .await?
        };
        Ok(cell.lock().await)
    }

    pub async fn get_prompt_template(
        &self,
        prompt_id: &str,
        prompt_variables: Option<&Object>,
    ) -> Result<(String, Object), BitBucketFailure> {
        // LiteLLM renders from the prompts already loaded, so this path never fetches a missing one
        let manager = self.prompt_manager().await?;
        render_template(&manager, prompt_id, prompt_variables)
    }

    pub async fn compile_prompt_helper(
        &self,
        prompt_id: Option<&str>,
        prompt_variables: Option<&Object>,
        _prompt_label: Option<&str>,
        _prompt_version: Option<i64>,
    ) -> Result<PromptManagementClient, BitBucketPromptError> {
        // LiteLLM compares the id against None, so an empty id still runs the manager.
        let Some(resolved_id) = prompt_id else {
            return Err(BitBucketPromptError::PromptIdRequired);
        };
        let failure = |source| BitBucketPromptError::Compile {
            prompt_id: resolved_id.to_owned(),
            source,
        };
        let (rendered, metadata) = {
            let mut manager = self.prompt_manager().await.map_err(failure)?;
            if !manager.prompts.contains_key(resolved_id) {
                manager
                    .load_prompt_from_bitbucket(resolved_id)
                    .await
                    .map_err(|source| failure(BitBucketFailure::Manager(source)))?;
            }
            let template =
                render_template(&manager, resolved_id, prompt_variables).map_err(failure)?;
            drop(manager);
            template
        };
        let messages = parse_prompt_to_messages(&rendered);
        let template_model = metadata
            .get("model")
            .and_then(Json::as_str)
            .map(str::to_owned);
        Ok(PromptManagementClient {
            prompt_id: Some(resolved_id.to_owned()),
            prompt_template: messages,
            prompt_template_model: template_model,
            prompt_optional_params: Some(project_optional_params(&metadata)),
            completed_messages: None,
        })
    }

    pub async fn async_compile_prompt_helper(
        &self,
        prompt_id: Option<&str>,
        prompt_variables: Option<&Object>,
        prompt_label: Option<&str>,
        prompt_version: Option<i64>,
    ) -> Result<PromptManagementClient, BitBucketPromptError> {
        // LiteLLM's async helper delegates to the sync one, and BitBucket has a single client here
        self.compile_prompt_helper(prompt_id, prompt_variables, prompt_label, prompt_version)
            .await
    }
}

fn render_template(
    manager: &BitBucketTemplateManager,
    prompt_id: &str,
    prompt_variables: Option<&Object>,
) -> Result<(String, Object), BitBucketFailure> {
    let Some(template) = manager.prompts.get(prompt_id) else {
        return Err(BitBucketFailure::TemplateNotFound {
            prompt_id: prompt_id.to_owned(),
        });
    };
    let empty = Object::new();
    let rendered = manager.render_template(prompt_id, prompt_variables.unwrap_or(&empty))?;
    Ok((rendered, template_metadata(template)))
}

fn rebuilt_config(config: &BitBucketConfig) -> BitBucketConfig {
    // BitBucketConfig derives no Clone, and the template manager takes its config by value
    let auth_method = config.auth_method.as_ref().map(|method| match method {
        AuthMethod::Basic => AuthMethod::Basic,
        AuthMethod::Token => AuthMethod::Token,
    });
    BitBucketConfig {
        workspace: config.workspace.clone(),
        repository: config.repository.clone(),
        access_token: config.access_token.clone(),
        branch: config.branch.clone(),
        base_url: config.base_url.clone(),
        auth_method,
        username: config.username.clone(),
        unrecognized: config.unrecognized.clone(),
    }
}
