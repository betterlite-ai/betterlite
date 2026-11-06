use betterlite_models::prompt_contracts::PromptParams;

use crate::arguments::json::Object;
use crate::gitlab_client::transport::GitLabTransport;
use crate::gitlab_templates::GitLabTemplateManager;
use crate::gitlab_templates::load;

use super::error::GetPromptError;
use super::params;

pub(crate) async fn prompt_template<T>(
    manager: &mut GitLabTemplateManager<T>,
    prompt_id: &str,
    prompt_variables: Option<&PromptParams>,
    ref_name: Option<&str>,
) -> Result<(String, Object), GetPromptError>
where
    T: GitLabTransport,
{
    if !manager.prompts.contains_key(prompt_id) {
        load::load_prompt(manager, prompt_id, ref_name).await?;
    }
    let Some(template) = manager.prompts.get(prompt_id) else {
        return Err(GetPromptError::TemplateNotFound {
            prompt_id: prompt_id.to_owned(),
        });
    };
    let metadata = params::prompt_metadata(template);
    // Prompt variables only feed the renderer, so a serde round trip reaches its Json type.
    let variables = match prompt_variables {
        Some(variables) => serde_json::from_value(serde_json::to_value(variables)?)?,
        None => Object::new(),
    };
    let rendered = manager.render_template(prompt_id, &variables)?;
    Ok((rendered, metadata))
}

mod tests;
