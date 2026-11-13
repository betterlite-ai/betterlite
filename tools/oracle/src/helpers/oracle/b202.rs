use std::collections::BTreeMap;

use betterlite_models::catalog::BundledCatalog;
use betterlite_models::provider_model_sets::{ProviderModelSet, ProviderModelSets};
use serde::Deserialize;

use crate::helpers::{HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    models: Vec<String>,
}

pub(crate) struct Op;

// main.py asks get_llm_provider for the model a request names, then switches a chat-catalog miss to text-completion-openai
fn selected_model(
    model: &str,
    chat_models: &std::collections::BTreeSet<String>,
    text_models: &std::collections::BTreeSet<String>,
) -> (String, String) {
    match model.split_once('/') {
        Some((provider, name)) => (provider.to_owned(), name.to_owned()),
        None if chat_models.contains(model) => (String::from("openai"), model.to_owned()),
        None if text_models.contains(model) => {
            (String::from("text-completion-openai"), model.to_owned())
        }
        None => (String::from("unmapped"), model.to_owned()),
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut sets = ProviderModelSets::default();
        sets.add_known_models(&BundledCatalog::default().get());
        let chat_models = sets
            .sets
            .get(&ProviderModelSet::OpenAiChatCompletion)
            .cloned()
            .unwrap_or_default();
        let text_models = sets
            .sets
            .get(&ProviderModelSet::OpenAiTextCompletion)
            .cloned()
            .unwrap_or_default();
        let mut selected = BTreeMap::new();
        for model in &input.models {
            let (provider, name) = selected_model(model, &chat_models, &text_models);
            let target = if provider == "openai" && !chat_models.contains(&name) {
                format!("text-completion-openai/{name}")
            } else {
                format!("{provider}/{name}")
            };
            selected.insert(model.clone(), target);
        }
        serde_json::to_value(selected).map_err(|error| {
            Box::new(HelperError {
                class: String::from("InvalidOutput"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
