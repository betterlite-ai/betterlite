use std::future::{Future, ready};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use betterlite_models::prompt_contracts::PromptManagementClient;
use thiserror::Error;

use crate::arguments::json::Object;
use crate::dotprompt_compilation::compile::{self, CompileError};
use crate::dotprompt_compilation::global::GlobalPromptDirectory;
use crate::dotprompt_templates::PromptManager;
use crate::dotprompt_templates::loading::PromptManagerError;

#[derive(Debug, Error)]
pub enum DotpromptManagerError {
    #[error(
        "Either prompt_directory or prompt_data must be set before using dotprompt manager. \
Set litellm.global_prompt_directory, initialize with prompt_directory parameter, or provide prompt_data."
    )]
    MissingPromptSource,
    #[error(transparent)]
    Build(#[from] PromptManagerError),
}

#[derive(Debug)]
pub enum PromptDataInput {
    Text(String),
    Object(Object),
}

#[derive(Debug)]
pub struct DotpromptManager {
    prompt_directory: Option<String>,
    prompt_file: Option<PathBuf>,
    prompt_data: Object,
    prompt_id: Option<String>,
    cache: Mutex<Option<PromptManager>>,
}

impl DotpromptManager {
    pub const INTEGRATION_NAME: &'static str = "dotprompt";

    pub fn new(
        prompt_directory: Option<String>,
        prompt_file: Option<PathBuf>,
        prompt_data: Option<PromptDataInput>,
        prompt_id: Option<String>,
        global_prompt_directory: &GlobalPromptDirectory,
    ) -> Result<Self, serde_json::Error> {
        let decoded_prompt_data = match prompt_data {
            Some(PromptDataInput::Text(text)) => serde_json::from_str(&text)?,
            Some(PromptDataInput::Object(object)) => object,
            None => Object::new(),
        };

        Ok(Self {
            prompt_directory: prompt_directory.or_else(|| global_prompt_directory.get()),
            prompt_file,
            prompt_data: decoded_prompt_data,
            prompt_id,
            cache: Mutex::new(None),
        })
    }

    pub(crate) fn with_prompt_manager<Output>(
        &self,
        action: impl FnOnce(&PromptManager) -> Output,
    ) -> Result<Output, DotpromptManagerError> {
        let mut guard = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.is_none() {
            if self.prompt_directory.is_none()
                && self.prompt_data.is_empty()
                && self.prompt_file.is_none()
            {
                return Err(DotpromptManagerError::MissingPromptSource);
            }
            let built = PromptManager::new(
                self.prompt_id.as_deref(),
                self.prompt_directory.as_deref().map(Path::new),
                Some(&self.prompt_data),
                self.prompt_file.as_deref(),
            )?;
            *guard = Some(built);
        }
        guard.as_ref().map_or_else(
            || Err(DotpromptManagerError::MissingPromptSource),
            |manager| Ok(action(manager)),
        )
    }

    // A lookup failure, including a missing prompt source, is swallowed as ineligible.
    pub fn should_run_prompt_management(&self, prompt_id: Option<&str>) -> bool {
        let Some(resolved_prompt_id) = prompt_id else {
            return false;
        };
        self.with_prompt_manager(|prompt_manager| {
            prompt_manager
                .get_prompt(resolved_prompt_id, None)
                .is_some()
        })
        .unwrap_or(false)
    }

    pub fn compile_prompt_helper(
        &self,
        prompt_id: Option<&str>,
        prompt_variables: Option<&Object>,
        version: Option<i64>,
    ) -> Result<PromptManagementClient, Box<CompileError>> {
        let Some(resolved_prompt_id) = prompt_id else {
            return Err(Box::new(CompileError::PromptIdRequired));
        };
        compile::compile_inner(self, resolved_prompt_id, prompt_variables, version).map_err(
            |source| {
                Box::new(CompileError::Compile {
                    prompt_id: resolved_prompt_id.to_owned(),
                    source,
                })
            },
        )
    }

    pub fn async_compile_prompt_helper(
        &self,
        prompt_id: Option<&str>,
        prompt_variables: Option<&Object>,
        version: Option<i64>,
    ) -> impl Future<Output = Result<PromptManagementClient, Box<CompileError>>> {
        if prompt_id.is_none() {
            ready(Err(Box::new(CompileError::PromptIdRequired)))
        } else {
            ready(self.compile_prompt_helper(prompt_id, prompt_variables, version))
        }
    }
}

mod tests;
