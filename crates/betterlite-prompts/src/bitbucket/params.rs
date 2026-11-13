use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::PromptParams;

use crate::arguments::json::{Json, Object};
use crate::bitbucket_prompt_manager::template::BitBucketPromptTemplate;

const OPTIONAL_PARAMS: [&str; 5] = [
    "temperature",
    "max_tokens",
    "top_p",
    "frequency_penalty",
    "presence_penalty",
];

#[must_use]
pub fn template_metadata(template: &BitBucketPromptTemplate) -> Object {
    let model = template
        .model
        .as_ref()
        .map_or(Json::Null, |model| Json::Text(model.clone()));
    let temperature = template
        .temperature
        .as_ref()
        .map_or(Json::Null, |number| Json::Number(number.clone()));
    let max_tokens = template
        .max_tokens
        .as_ref()
        .map_or(Json::Null, |number| Json::Number(number.clone()));
    let mut metadata = Object::new();
    metadata.insert("model".to_owned(), model);
    metadata.insert("temperature".to_owned(), temperature);
    metadata.insert("max_tokens".to_owned(), max_tokens);
    for (key, value) in &template.optional_params {
        metadata.insert(key.clone(), value.clone());
    }
    metadata
}

#[must_use]
pub fn project_optional_params(metadata: &Object) -> PromptParams {
    let mut params = PromptParams::new();
    for name in OPTIONAL_PARAMS {
        if let Some(value) = metadata.get(name) {
            params.insert(name.to_owned(), ContractJson::from(value));
        }
    }
    params
}
