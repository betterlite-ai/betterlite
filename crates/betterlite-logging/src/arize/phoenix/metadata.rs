use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::PromptParams;
use indexmap::IndexMap;

use crate::arize::phoenix::parsing::named_provider;
use crate::arize::phoenix::records::PromptTemplate;

const OPTIONAL_PARAMETERS: [&str; 5] = [
    "temperature",
    "max_tokens",
    "top_p",
    "frequency_penalty",
    "presence_penalty",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledSettings {
    pub model: Option<String>,
    pub optional_params: PromptParams,
}

// A key the provider block repeats from model, temperature or max_tokens keeps the template's own value.
pub fn template_metadata(template: &PromptTemplate) -> IndexMap<String, Json> {
    let metadata = &template.metadata;
    let mut entries = IndexMap::new();
    entries.insert(
        "model".to_owned(),
        metadata.model_name.clone().map_or(Json::Null, Json::Text),
    );
    entries.insert(
        "temperature".to_owned(),
        metadata
            .temperature
            .clone()
            .map_or(Json::Null, Json::Number),
    );
    entries.insert(
        "max_tokens".to_owned(),
        metadata.max_tokens.clone().map_or(Json::Null, Json::Number),
    );
    if let Some((_, Json::Object(parameters))) = named_provider(&metadata.invocation_parameters) {
        for (name, value) in parameters {
            entries.entry(name.clone()).or_insert_with(|| value.clone());
        }
    }
    entries
}

#[must_use]
pub fn compiled_settings(template: &PromptTemplate) -> CompiledSettings {
    let metadata = template_metadata(template);
    let model = match metadata.get("model") {
        Some(Json::Text(name)) => Some(name.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    };
    let mut optional_params = PromptParams::new();
    for name in OPTIONAL_PARAMETERS {
        if let Some(value) = metadata.get(name) {
            optional_params.insert(name.to_owned(), value.clone());
        }
    }
    CompiledSettings {
        model,
        optional_params,
    }
}

mod tests;
