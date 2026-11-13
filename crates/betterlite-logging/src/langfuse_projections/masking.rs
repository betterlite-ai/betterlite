use std::fmt::Display;

use betterlite_models::decisions::Json;

// litellm.integrations.langfuse.LangFuseLogger._apply_masking_function; the callable's failure is caught per node so logging never stops
#[must_use]
pub fn apply_masking_function<E>(
    data: &Json,
    masking_function: &dyn Fn(&Json) -> Result<Json, E>,
) -> Json
where
    E: Display,
{
    match data {
        Json::Null => Json::Null,
        Json::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), apply_masking_function(value, masking_function)))
                .collect(),
        ),
        Json::List(items) => Json::List(
            items
                .iter()
                .map(|item| apply_masking_function(item, masking_function))
                .collect(),
        ),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) => match masking_function(data) {
            Ok(masked) => masked,
            Err(error) => {
                tracing::warn!(
                    "Failed to apply masking function: {error}. Returning original data."
                );
                data.clone()
            }
        },
    }
}
