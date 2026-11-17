use betterlite_anthropic::params::error::ParamsError;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;

// The parameter tree as the prompt tree the beta feature detectors read; the two carry the same shapes.
pub(crate) fn prompt_json(value: &Json) -> PromptJson {
    match value {
        Json::Null => PromptJson::Null,
        Json::Bool(flag) => PromptJson::Bool(*flag),
        Json::Number(number) => PromptJson::Number(number.clone()),
        Json::Text(text) => PromptJson::Text(text.clone()),
        Json::List(items) => PromptJson::List(items.iter().map(prompt_json).collect()),
        Json::Object(fields) => PromptJson::Object(
            fields
                .iter()
                .map(|(name, entry)| (name.clone(), prompt_json(entry)))
                .collect(),
        ),
    }
}

// A parameter the Anthropic schema declares as a list of objects; an absent or null one reads as none.
pub(crate) fn prompt_list(
    params: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Vec<PromptJson>, ParamsError> {
    match params.get(field) {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(items)) => Ok(items.iter().map(prompt_json).collect()),
        Some(_) => Err(ParamsError::NotList { field }),
    }
}
