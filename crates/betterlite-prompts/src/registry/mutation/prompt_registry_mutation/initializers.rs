use betterlite_models::prompt_contracts::PromptManagementClient;
use betterlite_models::prompt_spec::{PromptLiteLLMParams, PromptSpec};
use indexmap::IndexMap;

use crate::dotprompt_templates::version::strip_version_suffix;

pub type PromptInitializer = fn(&PromptLiteLLMParams, &PromptSpec) -> PromptManagementClient;

// The closed set LiteLLM discovers by importing every integrations package; a new backend is one line here and a recompile
pub const PROMPT_INITIALIZER_REGISTRY: [(&str, PromptInitializer); 4] = [
    ("dotprompt", dotprompt_initializer),
    ("bitbucket", bitbucket_initializer),
    ("gitlab", gitlab_initializer),
    ("generic_prompt_management", generic_initializer),
];

// LiteLLM's get_prompt_initializer_from_integrations, shortened to the 30-character name rule
#[must_use]
pub fn discover_prompt_initializers() -> IndexMap<&'static str, PromptInitializer> {
    PROMPT_INITIALIZER_REGISTRY.iter().copied().collect()
}

#[must_use]
pub fn prompt_initializer(integration: &str) -> Option<PromptInitializer> {
    PROMPT_INITIALIZER_REGISTRY
        .iter()
        .find(|(name, _)| *name == integration)
        .map(|(_, initializer)| *initializer)
}

// A registered client holds no compiled template yet, so the initializers fill the id the backend would fetch
const fn registered_client(prompt_id: Option<String>) -> PromptManagementClient {
    PromptManagementClient {
        prompt_id,
        prompt_template: Vec::new(),
        prompt_template_model: None,
        prompt_optional_params: None,
        completed_messages: None,
    }
}

// LiteLLM's dotprompt initializer keeps the params id, else the version-stripped spec id, else the spec id
fn dotprompt_initializer(
    params: &PromptLiteLLMParams,
    prompt: &PromptSpec,
) -> PromptManagementClient {
    let registered_id = params
        .prompt_id
        .clone()
        .or_else(|| strip_version_suffix(&prompt.prompt_id).map(str::to_owned))
        .unwrap_or_else(|| prompt.prompt_id.clone());
    registered_client(Some(registered_id))
}

fn bitbucket_initializer(
    params: &PromptLiteLLMParams,
    _prompt: &PromptSpec,
) -> PromptManagementClient {
    registered_client(params.prompt_id.clone())
}

// LiteLLM's gitlab initializer maps the spec's own prompt id to the file path under the repository
fn gitlab_initializer(
    _params: &PromptLiteLLMParams,
    prompt: &PromptSpec,
) -> PromptManagementClient {
    registered_client(Some(prompt.prompt_id.clone()))
}

fn generic_initializer(
    params: &PromptLiteLLMParams,
    _prompt: &PromptSpec,
) -> PromptManagementClient {
    registered_client(params.prompt_id.clone())
}
