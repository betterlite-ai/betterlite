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

pub(crate) fn project_optional_params(
    metadata: &Object,
) -> Result<PromptParams, serde_json::Error> {
    let mut projected = PromptParams::new();
    for name in PROJECTED_PARAMS {
        if let Some(value) = metadata.get(name) {
            projected.insert(
                name.to_owned(),
                serde_json::from_value(serde_json::to_value(value)?)?,
            );
        }
    }
    Ok(projected)
}

mod tests;
