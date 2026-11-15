use std::collections::BTreeSet;

use betterlite_models::catalog::BundledCatalog;
use betterlite_models::provider_model_sets::{ProviderModelSet, ProviderModelSets};

use crate::chat::deployment::{ChatDeployment, Provider};

// The models LiteLLM's open_ai_chat_completion_models and open_ai_text_completion_models hold, as B193 builds them
#[derive(Debug, Clone, Default)]
pub struct CompletionCatalog {
    pub chat_models: BTreeSet<String>,
    pub text_models: BTreeSet<String>,
}

impl CompletionCatalog {
    #[must_use]
    pub fn bundled() -> Self {
        let mut sets = ProviderModelSets::default();
        sets.add_known_models(&BundledCatalog::default().get());
        Self {
            chat_models: sets
                .sets
                .remove(&ProviderModelSet::OpenAiChatCompletion)
                .unwrap_or_default(),
            text_models: sets
                .sets
                .remove(&ProviderModelSet::OpenAiTextCompletion)
                .unwrap_or_default(),
        }
    }
}

// Where main.py sends a completion request after its prompt dispatch and provider switch
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionRoute {
    Text { model: String },
    // a model in the chat catalog stays with the chat provider; B203 owns that bridge
    Chat { model: String },
    Unsupported { provider: String },
}

#[must_use]
pub fn route_for(deployment: &ChatDeployment, catalog: &CompletionCatalog) -> CompletionRoute {
    match &deployment.provider {
        Provider::OpenAi { model } => {
            let model = model.clone();
            if catalog.chat_models.contains(&model) {
                CompletionRoute::Chat { model }
            } else {
                CompletionRoute::Text { model }
            }
        }
        Provider::Unsupported { model } => match model.strip_prefix("text-completion-openai/") {
            // main.py consults the chat catalog only for a bare openai model, so an explicit prefix always means the text endpoint
            Some(bare) => CompletionRoute::Text {
                model: bare.to_owned(),
            },
            None if catalog.text_models.contains(model) => CompletionRoute::Text {
                model: model.clone(),
            },
            None => CompletionRoute::Unsupported {
                provider: deployment.litellm_model.clone(),
            },
        },
    }
}
