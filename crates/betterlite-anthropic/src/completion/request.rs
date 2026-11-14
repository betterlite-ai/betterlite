use betterlite_models::decisions::Json;
use betterlite_openai::base_helpers::provider_config;
use betterlite_prompts::builtin::Message;
use indexmap::IndexMap;

use crate::completion::error::CompletionError;
use crate::completion::options::OptionDefaults;
use crate::completion::prompt::{PromptSource, prompt_from_messages};

// model and prompt come first, so an optional param of the same name overrides them
pub async fn transform_request(
    source: &PromptSource<'_>,
    model: &str,
    messages: &[Message],
    mut optional_params: IndexMap<String, Json>,
    defaults: &OptionDefaults,
) -> Result<IndexMap<String, Json>, CompletionError> {
    let prompt = prompt_from_messages(source, model, messages).await?;
    // a request's own value beats the configured default, so only absent keys are filled
    let configured =
        provider_config(defaults).map_err(|error| CompletionError::Config(Box::new(error)))?;
    for (key, value) in configured {
        if !optional_params.contains_key(&key) {
            let converted = serde_json::from_value(value)
                .map_err(|error| CompletionError::Json { source: error })?;
            optional_params.insert(key, converted);
        }
    }
    let mut data = IndexMap::from([
        ("model".to_owned(), Json::Text(model.to_owned())),
        ("prompt".to_owned(), Json::Text(prompt)),
    ]);
    data.extend(optional_params);
    Ok(data)
}
