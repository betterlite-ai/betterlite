use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::PromptParams;

use crate::arguments::json::{Json, Object};
use crate::gitlab_templates::template::GitLabPromptTemplate;

const PROJECTED_PARAMS: [&str; 5] = [
    "temperature",
    "max_tokens",
    "top_p",
    "frequency_penalty",
    "presence_penalty",
];

#[must_use]
pub(crate) fn prompt_metadata(template: &GitLabPromptTemplate) -> Object {
    let mut metadata = Object::new();
    metadata.insert(
        "model".to_owned(),
        template
            .model
            .as_deref()
            .map_or(Json::Null, |model| Json::Text(model.to_owned())),
    );
    metadata.insert(
        "temperature".to_owned(),
        template.temperature.clone().unwrap_or(Json::Null),
    );
    metadata.insert(
        "max_tokens".to_owned(),
        template.max_tokens.clone().unwrap_or(Json::Null),
    );
    for (key, value) in &template.optional_params {
        metadata.insert(key.clone(), value.clone());
    }
    metadata
}

#[must_use]
pub(crate) fn project_optional_params(metadata: &Object) -> PromptParams {
    let mut projected = PromptParams::new();
    for name in PROJECTED_PARAMS {
        if let Some(value) = metadata.get(name) {
            projected.insert(name.to_owned(), to_contract_json(value));
        }
    }
    projected
}

fn to_contract_json(value: &Json) -> ContractJson {
    match value {
        Json::Null => ContractJson::Null,
        Json::Bool(flag) => ContractJson::Bool(*flag),
        Json::Number(number) => ContractJson::Number(number.clone()),
        Json::Text(text) => ContractJson::Text(text.clone()),
        Json::List(items) => ContractJson::List(items.iter().map(to_contract_json).collect()),
        Json::Object(fields) => ContractJson::Object(
            fields
                .iter()
                .map(|(key, field_value)| (key.clone(), to_contract_json(field_value)))
                .collect(),
        ),
    }
}

mod tests;
