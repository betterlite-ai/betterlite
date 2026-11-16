use std::cmp::Reverse;

use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::dotprompt_templates::content::dotprompt_content_to_json;
use betterlite_prompts::registry::prompt_registry_helpers::{
    get_base_prompt_id, get_version_number,
};

use super::PromptReadState;
use super::wire::PromptTemplateBase;

pub(super) fn not_found_detail(prompt_id: &str, environment: Option<&str>) -> String {
    let Some(wanted) = environment else {
        return format!("Prompt {prompt_id} not found");
    };
    format!("Prompt {prompt_id} not found in environment {wanted}")
}

pub(super) fn filter_environment(
    prompts: Vec<PromptSpec>,
    environment: Option<&str>,
) -> Vec<PromptSpec> {
    let Some(wanted) = environment else {
        return prompts;
    };
    prompts
        .into_iter()
        .filter(|spec| spec.environment == wanted)
        .collect()
}

pub(super) async fn prompt_specs<Table>(state: &PromptReadState<Table>) -> Vec<PromptSpec> {
    let registry = state.prompts.read().await;
    registry.in_memory_prompts.values().cloned().collect()
}

// LiteLLM serves history from the registry, newest first, when the proxy has no database
pub(super) async fn in_memory_versions<Table>(
    state: &PromptReadState<Table>,
    base_prompt_id: &str,
    environment: Option<&str>,
) -> Vec<PromptSpec> {
    let mut versions: Vec<PromptSpec> = {
        let registry = state.prompts.read().await;
        registry
            .in_memory_prompts
            .values()
            .filter(|spec| get_base_prompt_id(&spec.prompt_id) == base_prompt_id)
            .filter(|spec| environment.is_none_or(|wanted| spec.environment == wanted))
            .cloned()
            .collect()
    };
    versions.sort_by_key(|spec| Reverse(get_version_number(&spec.prompt_id)));
    versions
}

// A dotprompt parse failure yields no template, and DotpromptManager.prompt_manager's templates need a public accessor (B4052)
pub(super) fn prompt_template(
    spec: &PromptSpec,
    base_prompt_id: &str,
) -> Option<PromptTemplateBase> {
    let parsed =
        dotprompt_content_to_json(spec.litellm_params.dotprompt_content.as_deref()?).ok()?;
    Some(PromptTemplateBase {
        litellm_prompt_id: base_prompt_id.to_owned(),
        content: parsed.get("content")?.as_str()?.to_owned(),
        metadata: parsed.get("metadata")?.as_object().cloned(),
    })
}
