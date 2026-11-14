use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::helpers::{HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    chat_models: BTreeSet<String>,
    text_models: BTreeSet<String>,
    models: Vec<String>,
}

pub(crate) struct Op;

// main.py asks get_llm_provider for the model a request names, then switches an openai miss of the chat catalog to text-completion-openai
fn selected_model(
    model: &str,
    chat_models: &BTreeSet<String>,
    text_models: &BTreeSet<String>,
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
        let mut selected = BTreeMap::new();
        for model in &input.models {
            let (provider, name) = selected_model(model, &input.chat_models, &input.text_models);
            let target = if provider == "openai" && !input.chat_models.contains(&name) {
                format!("text-completion-openai/{name}")
            } else {
                model.clone()
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
