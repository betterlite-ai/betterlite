use crate::gitlab_client::transport::GitLabTransport;
use crate::gitlab_templates::GitLabTemplateManager;
use crate::gitlab_templates::error::ManagerError;
use crate::gitlab_templates::load;

use super::init::manager::GitLabPromptManager;

pub struct TemplateSlot<T> {
    pub manager: GitLabTemplateManager<T>,
    pub configured_loaded: bool,
}

pub(crate) async fn ensure_configured_loaded<T>(
    prompt_manager: &GitLabPromptManager<T>,
    slot: &mut TemplateSlot<T>,
) -> Result<(), ManagerError>
where
    T: GitLabTransport,
{
    if slot.configured_loaded {
        return Ok(());
    }
    if let Some(initial) = prompt_manager
        .prompt_id
        .as_deref()
        .filter(|id| !id.is_empty())
    {
        load::load_prompt(
            &mut slot.manager,
            initial,
            prompt_manager.ref_override.as_deref(),
        )
        .await?;
    }
    slot.configured_loaded = true;
    Ok(())
}

mod tests;
