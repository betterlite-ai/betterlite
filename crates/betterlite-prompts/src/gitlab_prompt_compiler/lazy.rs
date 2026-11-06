use std::sync::PoisonError;

use crate::gitlab_client::transport::GitLabTransport;
use crate::gitlab_templates::GitLabTemplateManager;
use crate::gitlab_templates::error::ManagerError;
use crate::gitlab_templates::load;
use crate::gitlab_templates::manager_init::{self, ClientSource};
use crate::gitlab_templates::parse::FrontmatterParser;

use super::init::manager::GitLabPromptManager;

pub(crate) struct TemplateSlot<T> {
    pub(crate) manager: Option<GitLabTemplateManager<T>>,
    pub(crate) configured_loaded: bool,
}

pub(crate) async fn ensure_ready<T>(
    prompt_manager: &GitLabPromptManager<T>,
    slot: &mut TemplateSlot<T>,
) -> Result<(), ManagerError>
where
    T: GitLabTransport,
{
    if slot.manager.is_none() {
        let pending = prompt_manager
            .pending_client
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(client) = pending {
            let built = manager_init::build(
                &prompt_manager.gitlab_config,
                None,
                prompt_manager.ref_override.as_deref(),
                ClientSource::Provided(client),
                FrontmatterParser::Yaml,
            )
            .await?;
            slot.manager = Some(built);
        }
    }
    if !slot.configured_loaded {
        if let Some(initial) = prompt_manager
            .prompt_id
            .as_deref()
            .filter(|id| !id.is_empty())
            && let Some(manager) = slot.manager.as_mut()
        {
            load::load_prompt(manager, initial, None).await?;
        }
        slot.configured_loaded = true;
    }
    Ok(())
}

mod tests;
