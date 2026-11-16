use betterlite_models::prompt_contracts::PromptManagementClient;

// LiteLLM calls litellm.logging_callback_manager; that registry holds callbacks it runs itself, so the prompt registry takes the same two operations as a port
pub trait PromptCallbackManager {
    fn add_litellm_callback(&mut self, callback: &PromptManagementClient);

    fn remove_callback_from_all_lists(&mut self, callback: &PromptManagementClient);
}
