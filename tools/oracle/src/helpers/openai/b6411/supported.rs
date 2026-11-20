use std::collections::BTreeSet;

use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_openai::sail_chat::supported_params;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Steps {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    model: String,
    chat_models: Vec<String>,
    text_models: Vec<String>,
}

pub(super) fn run(steps: &Steps) -> Value {
    let mut records = Vec::with_capacity(steps.steps.len());
    for step in &steps.steps {
        let catalog = CatalogCapabilities {
            chat_models: step
                .chat_models
                .iter()
                .cloned()
                .collect::<BTreeSet<String>>(),
            text_models: step
                .text_models
                .iter()
                .cloned()
                .collect::<BTreeSet<String>>(),
        };
        let supported = supported_params(&step.model, &catalog);
        records.push(json!({"ok": true, "value": supported, "status": null}));
    }
    Value::Array(records)
}
