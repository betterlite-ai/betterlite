use std::sync::Mutex;

use betterlite_models::prompt_contracts::{
    PromptCompileRequest, PromptManagementClient, PromptParams,
};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::MutexGuard;

use crate::arguments::json::Object;
use crate::gitlab_client::GitLabClient;
use crate::gitlab_client::config::GitLabConfig;
use crate::gitlab_client::error::GitLabClientError;
use crate::gitlab_client::transport::GitLabTransport;
use crate::gitlab_templates::error::ManagerError;

use super::super::compile;
use super::super::error::{CompileError, GetPromptError};
use super::super::get;
use super::super::lazy::{self, TemplateSlot};

pub struct GitLabPromptManager<T> {
    pub(crate) gitlab_config: GitLabConfig,
    pub(crate) prompt_id: Option<String>,
    pub(crate) ref_override: Option<String>,
    pub(crate) pending_client: Mutex<Option<GitLabClient<T>>>,
    pub(crate) template_manager: AsyncMutex<TemplateSlot<T>>,
}

impl<T> GitLabPromptManager<T>
where
    T: GitLabTransport,
{
    pub const INTEGRATION_NAME: &'static str = "gitlab";

    // LiteLLM's eager branch builds its template manager without the injected client.
    pub fn new(
        gitlab_config: GitLabConfig,
        prompt_id: Option<String>,
        ref_override: Option<String>,
        injected_client: Option<GitLabClient<T>>,
        transport: T,
    ) -> Result<Self, GitLabClientError> {
        let configured = prompt_id.as_deref().is_some_and(|id| !id.is_empty());
        let mut client = match injected_client {
            Some(client) if !configured => client,
            Some(_) | None => GitLabClient::new(&gitlab_config, transport)?,
        };
        if let Some(requested) = ref_override.as_deref().filter(|name| !name.is_empty()) {
            client.set_ref(requested)?;
        }
        Ok(Self {
            gitlab_config,
            prompt_id,
            ref_override,
            pending_client: Mutex::new(Some(client)),
            template_manager: AsyncMutex::new(TemplateSlot {
                manager: None,
                configured_loaded: false,
            }),
        })
    }

    pub(crate) async fn prompt_manager(
        &self,
    ) -> Result<MutexGuard<'_, TemplateSlot<T>>, ManagerError> {
        let mut slot = self.template_manager.lock().await;
        lazy::ensure_ready(self, &mut slot).await?;
        Ok(slot)
    }

    pub async fn get_prompt_template(
        &self,
        prompt_id: &str,
        prompt_variables: Option<&PromptParams>,
        ref_name: Option<&str>,
    ) -> Result<(String, Object), GetPromptError> {
        let mut slot = self.prompt_manager().await?;
        get::prompt_template(
            slot.manager.as_mut().ok_or(GetPromptError::Unavailable)?,
            prompt_id,
            prompt_variables,
            ref_name,
        )
        .await
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
            slot.manager.as_mut().ok_or(GetPromptError::Unavailable)?,
            prompt_id,
            request.prompt_variables,
            request.dynamic_callback_params,
        )
        .await
    }
}

mod tests;
