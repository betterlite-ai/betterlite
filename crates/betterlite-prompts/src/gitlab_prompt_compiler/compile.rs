use betterlite_models::prompt_contracts::{
    PromptManagementClient, PromptParams, StandardCallbackDynamicParams,
};

use crate::arguments::json::Json;
use crate::gitlab_client::transport::GitLabTransport;
use crate::gitlab_templates::GitLabTemplateManager;
use crate::gitlab_templates::encoding::decode_prompt_id;

use super::error::GetPromptError;
use super::get;
use super::params;
use super::parse;
use super::ref_override;

pub(crate) async fn compile_inner<T>(
    manager: &mut GitLabTemplateManager<T>,
    prompt_id: &str,
    prompt_variables: Option<&PromptParams>,
    dynamic_callback_params: &StandardCallbackDynamicParams,
) -> Result<PromptManagementClient, GetPromptError>
where
    T: GitLabTransport,
{
    ref_override::ensure_loaded(
        manager,
        &decode_prompt_id(prompt_id),
        dynamic_callback_params,
    )
    .await?;
    let (rendered, metadata) =
        get::prompt_template(manager, prompt_id, prompt_variables, None).await?;
    Ok(PromptManagementClient {
        prompt_id: Some(prompt_id.to_owned()),
        prompt_template: parse::parse_prompt_to_messages(&rendered),
        prompt_template_model: metadata
            .get("model")
            .and_then(Json::as_str)
            .map(str::to_owned),
        prompt_optional_params: Some(params::project_optional_params(&metadata)),
        completed_messages: None,
    })
}

mod tests;
