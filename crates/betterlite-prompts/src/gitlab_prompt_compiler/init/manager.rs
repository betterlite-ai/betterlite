use betterlite_models::prompt_contracts::{
    PromptCompileRequest, PromptManagementClient, PromptParams,
};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::MutexGuard;

use crate::arguments::json::Object;
use crate::gitlab_client::GitLabClient;
use crate::gitlab_client::config::GitLabConfig;
use crate::gitlab_client::transport::GitLabTransport;
use crate::gitlab_templates::error::ManagerError;
use crate::gitlab_templates::manager_init::{self, ClientSource};
use crate::gitlab_templates::parse::FrontmatterParser;

use super::super::compile;
use super::super::error::{CompileError, GetPromptError};
use super::super::get;
use super::super::lazy::{self, TemplateSlot};

pub struct GitLabPromptManager<T> {
    pub gitlab_config: GitLabConfig,
    pub prompt_id: Option<String>,
    pub ref_override: Option<String>,
    pub template_manager: AsyncMutex<TemplateSlot<T>>,
}

impl<T> GitLabPromptManager<T>
where
    T: GitLabTransport,
{
    pub const INTEGRATION_NAME: &'static str = "gitlab";

    // LiteLLM's eager branch builds its template manager without the injected client.
    pub async fn new(
        gitlab_config: GitLabConfig,
        prompt_id: Option<String>,
        ref_override: Option<String>,
        injected_client: Option<GitLabClient<T>>,
        transport: T,
    ) -> Result<Self, ManagerError> {
        let configured = prompt_id.as_deref().is_some_and(|id| !id.is_empty());
        let source = match injected_client {
            Some(client) if !configured => ClientSource::Provided(client),
            Some(_) | None => ClientSource::Transport(transport),
        };
        let manager = manager_init::build(
            &gitlab_config,
            None,
            ref_override.as_deref(),
            source,
            FrontmatterParser::Yaml,
        )
        .await?;
        Ok(Self {
            gitlab_config,
            prompt_id,
            ref_override,
            template_manager: AsyncMutex::new(TemplateSlot {
                manager,
                configured_loaded: false,
            }),
        })
    }

    pub async fn prompt_manager(&self) -> Result<MutexGuard<'_, TemplateSlot<T>>, ManagerError> {
        let mut slot = self.template_manager.lock().await;
        lazy::ensure_configured_loaded(self, &mut slot).await?;
        Ok(slot)
    }

    pub async fn get_prompt_template(
        &self,
        prompt_id: &str,
        prompt_variables: Option<&PromptParams>,
        ref_name: Option<&str>,
    ) -> Result<(String, Object), GetPromptError> {
        match self.prompt_manager().await {
            Ok(mut slot) => {
                get::prompt_template(&mut slot.manager, prompt_id, prompt_variables, ref_name).await
            }
            Err(source) => Err(GetPromptError::Manager(source)),
        }
    }

    pub async fn compile_prompt_helper(
        &self,
        request: &PromptCompileRequest<'_>,
    ) -> Result<PromptManagementClient, Box<CompileError>> {
        let prompt_id = request
            .prompt_id
            .ok_or_else(|| Box::new(CompileError::PromptIdRequired))?;
        match self.compile_locked(prompt_id, request).await {
            Ok(client) => Ok(client),
            Err(source) => Err(Box::new(CompileError::Compile {
                prompt_id: prompt_id.to_owned(),
                source,
            })),
        }
    }

    async fn compile_locked(
        &self,
        prompt_id: &str,
        request: &PromptCompileRequest<'_>,
    ) -> Result<PromptManagementClient, GetPromptError> {
        let mut slot = self.prompt_manager().await?;
        compile::compile_inner(
            &mut slot.manager,
            prompt_id,
            request.prompt_variables,
            request.dynamic_callback_params,
        )
        .await
    }
}

mod tests;
