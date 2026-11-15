use betterlite_models::prompt_contracts::PromptManagementClient;
use betterlite_models::prompt_spec::{PromptLiteLLMParams, PromptSpec};
use indexmap::IndexMap;

use crate::dotprompt_templates::version::strip_version_suffix;

// The closed set LiteLLM discovers by importing every integrations package; a new backend is one variant, one table line and a recompile
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptBackend {
    DotPrompt,
    Bitbucket,
    Gitlab,
    GenericPromptManagement,
}

// LiteLLM's prompt_initializer_registry, keyed by the name each integrations package registers
pub const PROMPT_INITIALIZER_REGISTRY: [(&str, PromptBackend); 4] = [
    ("dotprompt", PromptBackend::DotPrompt),
    ("bitbucket", PromptBackend::Bitbucket),
    ("gitlab", PromptBackend::Gitlab),
    (
        "generic_prompt_management",
        PromptBackend::GenericPromptManagement,
    ),
];

// LiteLLM's get_prompt_initializer_from_integrations, shortened to the 30-character name rule
#[must_use]
pub fn discover_prompt_initializers() -> IndexMap<&'static str, PromptBackend> {
    let mut discovered = IndexMap::with_capacity(PROMPT_INITIALIZER_REGISTRY.len());
    for (name, backend) in PROMPT_INITIALIZER_REGISTRY {
        discovered.insert(name, backend);
    }
    discovered
}

#[must_use]
pub fn prompt_initializer(integration: &str) -> Option<PromptBackend> {
    PROMPT_INITIALIZER_REGISTRY
        .iter()
        .find(|(name, _)| *name == integration)
        .map(|(_, backend)| *backend)
}

impl PromptBackend {
    // A registered client holds no compiled template yet, so each backend fills the id it would fetch
    #[must_use]
    pub fn initializer(
        self,
        params: &PromptLiteLLMParams,
        prompt: &PromptSpec,
    ) -> PromptManagementClient {
        match self {
            Self::DotPrompt => {
                let registered_id = params
                    .prompt_id
                    .clone()
                    .or_else(|| strip_version_suffix(&prompt.prompt_id).map(str::to_owned))
                    .unwrap_or_else(|| prompt.prompt_id.clone());
                registered_client(Some(registered_id))
            }
            Self::Bitbucket | Self::GenericPromptManagement => {
                registered_client(params.prompt_id.clone())
            }
            Self::Gitlab => registered_client(Some(prompt.prompt_id.clone())),
        }
    }
}

const fn registered_client(prompt_id: Option<String>) -> PromptManagementClient {
    PromptManagementClient {
        prompt_id,
        prompt_template: Vec::new(),
        prompt_template_model: None,
        prompt_optional_params: None,
        completed_messages: None,
    }
}
