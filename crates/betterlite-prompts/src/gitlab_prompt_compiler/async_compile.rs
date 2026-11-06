use betterlite_models::prompt_contracts::{
    PromptAsyncCompileRequest, PromptCompileRequest, PromptManagementClient,
};

use crate::gitlab_client::transport::GitLabTransport;

use super::error::CompileError;
use super::init::manager::GitLabPromptManager;

pub async fn async_compile_prompt_helper<T>(
    prompt_manager: &GitLabPromptManager<T>,
    request: &PromptAsyncCompileRequest<'_>,
) -> Result<PromptManagementClient, Box<CompileError>>
where
    T: GitLabTransport,
{
    if request.prompt_id.is_none() {
        return Err(Box::new(CompileError::PromptIdRequired));
    }
    let delegated = sync_request(request);
    prompt_manager.compile_prompt_helper(&delegated).await
}

#[must_use]
const fn sync_request<'request>(
    request: &'request PromptAsyncCompileRequest<'_>,
) -> PromptCompileRequest<'request> {
    PromptCompileRequest {
        prompt_id: request.prompt_id,
        prompt_spec: request.prompt_spec,
        prompt_variables: request.prompt_variables,
        dynamic_callback_params: request.dynamic_callback_params,
        prompt_label: request.prompt_label,
        prompt_version: request.prompt_version,
    }
}

mod tests;
