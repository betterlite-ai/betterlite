use betterlite_models::prompt_contracts::{PromptManagementClient, PromptParams};
use minijinja::Environment;
use thiserror::Error;

use crate::arize::phoenix::caching::{LoadError, TemplateCache};
use crate::arize::phoenix::client::PhoenixClient;
use crate::arize::phoenix::metadata::compiled_settings;
use crate::arize::phoenix::rendering::{RenderError, render_messages};

pub mod initializer;

#[derive(Debug, Error)]
pub enum CompileError {
    #[error("prompt_id is required for Arize Phoenix prompt manager")]
    PromptIdRequired,
    #[error("Error compiling prompt '{prompt_id}': {source}")]
    Load {
        prompt_id: String,
        #[source]
        source: LoadError,
    },
    #[error("Error compiling prompt '{prompt_id}': {source}")]
    Render {
        prompt_id: String,
        #[source]
        source: RenderError,
    },
}

#[derive(Debug)]
pub struct PhoenixPromptManager {
    pub client: PhoenixClient,
    pub prompt_id: Option<String>,
    pub templates: TemplateCache,
    pub environment: Environment<'static>,
}

impl PhoenixPromptManager {
    pub const INTEGRATION_NAME: &'static str = "arize";

    pub async fn compile_prompt_helper(
        &self,
        requested_id: Option<&str>,
        prompt_variables: Option<&PromptParams>,
    ) -> Result<PromptManagementClient, CompileError> {
        let Some(prompt_id) = requested_id else {
            return Err(CompileError::PromptIdRequired);
        };
        let load_failed = |source| CompileError::Load {
            prompt_id: prompt_id.to_owned(),
            source,
        };
        // LiteLLM builds its template manager on first use, and that build fetches the configured prompt.
        if let Some(configured) = self.prompt_id.as_deref() {
            self.templates
                .load(&self.client, configured)
                .await
                .map_err(load_failed)?;
        }
        let template = self
            .templates
            .load(&self.client, prompt_id)
            .await
            .map_err(load_failed)?;
        let messages =
            render_messages(&self.environment, &template, prompt_variables).map_err(|source| {
                CompileError::Render {
                    prompt_id: prompt_id.to_owned(),
                    source,
                }
            })?;
        let settings = compiled_settings(&template);
        Ok(PromptManagementClient {
            prompt_id: Some(prompt_id.to_owned()),
            prompt_template: messages,
            prompt_template_model: settings.model,
            prompt_optional_params: Some(settings.optional_params),
            completed_messages: None,
        })
    }
}

mod tests;
