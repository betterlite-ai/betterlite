use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;
use betterlite_models::prompt_spec::PromptSpec;

#[must_use]
pub const fn should_run_prompt_management(
    prompt_id: Option<&str>,
    _prompt_spec: Option<&PromptSpec>,
    _dynamic_callback_params: &StandardCallbackDynamicParams,
) -> bool {
    // LiteLLM compares the id against None, so an empty id still runs the manager.
    prompt_id.is_some()
}

mod tests;
