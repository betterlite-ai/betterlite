use std::future::Future;
use std::pin::Pin;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::decisions::Json;
use crate::prompt_spec::PromptSpec;

pub type AllMessageValues = Json;
pub type StandardCallbackDynamicParams = Json;
pub type PromptParams = IndexMap<String, Json>;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct PromptManagementClient {
    pub prompt_id: Option<String>,
    pub prompt_template: Vec<AllMessageValues>,
    pub prompt_template_model: Option<String>,
    pub prompt_optional_params: Option<PromptParams>,
    pub completed_messages: Option<Vec<AllMessageValues>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TestPromptRequest {
    pub dotprompt_content: String,
    pub prompt_variables: Option<PromptParams>,
    pub conversation_history: Option<Vec<IndexMap<String, String>>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PromptIgnoreFlags {
    pub model: bool,
    pub optional_params: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PromptContractError {
    #[error("custom prompt management does not support prompt compilation")]
    NotImplemented,
}

#[must_use]
pub fn resolve_ignore_flags(
    prompt_spec: Option<&PromptSpec>,
    requested: PromptIgnoreFlags,
) -> PromptIgnoreFlags {
    let params = prompt_spec.map(|spec| &spec.litellm_params);
    PromptIgnoreFlags {
        model: requested.model || params.is_some_and(|value| value.ignore_model),
        optional_params: requested.optional_params
            || params.is_some_and(|value| value.ignore_optional_params),
    }
}

#[must_use]
pub fn _get_model_from_prompt(
    prompt_management_client: &PromptManagementClient,
    integration_name: &str,
    model: &str,
) -> String {
    if let Some(template_model) = &prompt_management_client.prompt_template_model {
        return template_model.clone();
    }
    let prefix = format!("{integration_name}/");
    let mut resolved = model.to_owned();
    while let Some(stripped) = resolved.strip_prefix(&prefix) {
        resolved = stripped.to_owned();
    }
    resolved
}

const MODEL_RESOLVER: fn(&PromptManagementClient, &str, &str) -> String = _get_model_from_prompt;

#[must_use]
pub fn get_model_from_prompt(
    prompt_management_client: &PromptManagementClient,
    integration_name: &str,
    model: &str,
) -> String {
    let mut resolved = MODEL_RESOLVER(prompt_management_client, integration_name, model);
    resolved.shrink_to_fit();
    resolved
}

#[derive(Clone, Debug)]
pub struct PromptProcessingOptions {
    pub integration_name: String,
    pub ignore_flags: PromptIgnoreFlags,
}

#[must_use]
pub fn post_compile_prompt_processing(
    prompt_template: &PromptManagementClient,
    messages: Vec<AllMessageValues>,
    non_default_params: PromptParams,
    model: String,
    options: &PromptProcessingOptions,
) -> (String, Vec<AllMessageValues>, PromptParams) {
    let completed_messages = prompt_template
        .completed_messages
        .clone()
        .unwrap_or(messages);
    let mut updated_params = non_default_params;
    if !options.ignore_flags.optional_params
        && let Some(optional_params) = &prompt_template.prompt_optional_params
    {
        updated_params.extend(optional_params.clone());
    }
    let resolved_model = if options.ignore_flags.model {
        model
    } else {
        get_model_from_prompt(prompt_template, &options.integration_name, &model)
    };
    (resolved_model, completed_messages, updated_params)
}

pub struct PromptCompileRequest<'request> {
    pub prompt_id: Option<&'request str>,
    pub prompt_spec: Option<&'request PromptSpec>,
    pub prompt_variables: Option<&'request PromptParams>,
    pub dynamic_callback_params: &'request StandardCallbackDynamicParams,
    pub prompt_label: Option<&'request str>,
    pub prompt_version: Option<i64>,
}

pub struct PromptAsyncCompileRequest<'request> {
    pub prompt_id: Option<&'request str>,
    pub prompt_variables: Option<&'request PromptParams>,
    pub dynamic_callback_params: &'request StandardCallbackDynamicParams,
    pub prompt_spec: Option<&'request PromptSpec>,
    pub prompt_label: Option<&'request str>,
    pub prompt_version: Option<i64>,
}

pub trait PromptManagement {
    fn integration_name(&self) -> &'static str;
    fn should_run_prompt_management(
        &self,
        prompt_id: Option<&str>,
        prompt_spec: Option<&PromptSpec>,
        dynamic_callback_params: &StandardCallbackDynamicParams,
    ) -> bool;
    fn _compile_prompt_helper(
        &self,
        request: &PromptCompileRequest<'_>,
    ) -> Result<PromptManagementClient, PromptContractError>;
    fn async_compile_prompt_helper(
        &self,
        request: &PromptAsyncCompileRequest<'_>,
    ) -> Pin<
        Box<dyn Future<Output = Result<PromptManagementClient, PromptContractError>> + Send + '_>,
    >;
}

#[derive(Clone, Debug, Default)]
pub struct CustomPromptManagement;

impl PromptManagement for CustomPromptManagement {
    fn integration_name(&self) -> &'static str {
        "custom-prompt-management"
    }
    fn should_run_prompt_management(
        &self,
        _prompt_id: Option<&str>,
        _prompt_spec: Option<&PromptSpec>,
        _dynamic_callback_params: &StandardCallbackDynamicParams,
    ) -> bool {
        true
    }
    fn _compile_prompt_helper(
        &self,
        _request: &PromptCompileRequest<'_>,
    ) -> Result<PromptManagementClient, PromptContractError> {
        Err(PromptContractError::NotImplemented)
    }
    fn async_compile_prompt_helper(
        &self,
        _request: &PromptAsyncCompileRequest<'_>,
    ) -> Pin<
        Box<dyn Future<Output = Result<PromptManagementClient, PromptContractError>> + Send + '_>,
    > {
        Box::pin(async { Err(PromptContractError::NotImplemented) })
    }
}

mod tests;
