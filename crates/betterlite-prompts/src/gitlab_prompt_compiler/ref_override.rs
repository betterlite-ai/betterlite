use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;

use crate::gitlab_client::transport::GitLabTransport;
use crate::gitlab_templates::GitLabTemplateManager;
use crate::gitlab_templates::error::ManagerError;
use crate::gitlab_templates::load;

pub(crate) async fn ensure_loaded<T>(
    manager: &mut GitLabTemplateManager<T>,
    prompt_id: &str,
    dynamic_callback_params: &StandardCallbackDynamicParams,
) -> Result<(), ManagerError>
where
    T: GitLabTransport,
{
    if manager.prompts.contains_key(prompt_id) {
        return Ok(());
    }
    // LiteLLM reads extra.git_ref off the callback params, but its getattr can never see a TypedDict key.
    let extra = match dynamic_callback_params {
        ContractJson::Object(params) => params.get("extra"),
        ContractJson::Null
        | ContractJson::Bool(_)
        | ContractJson::Number(_)
        | ContractJson::Text(_)
        | ContractJson::List(_) => None,
    };
    let git_ref = match extra {
        Some(ContractJson::Object(fields)) => match fields.get("git_ref") {
            Some(ContractJson::Text(name)) => Some(name.as_str()),
            Some(
                ContractJson::Null
                | ContractJson::Bool(_)
                | ContractJson::Number(_)
                | ContractJson::List(_)
                | ContractJson::Object(_),
            )
            | None => None,
        },
        Some(
            ContractJson::Null
            | ContractJson::Bool(_)
            | ContractJson::Number(_)
            | ContractJson::Text(_)
            | ContractJson::List(_),
        )
        | None => None,
    };
    load::load_prompt(manager, prompt_id, git_ref).await
}

mod tests;
