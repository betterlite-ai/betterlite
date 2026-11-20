use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::spend_metadata::as_object;

// LiteLLM strips one leading "metadata." so both "foo" and "metadata.foo" address the same configured key
#[must_use]
pub fn custom_labels_from_metadata(
    metadata: &JsonObject,
    configured_keys: &[String],
) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    for original_key in configured_keys {
        let key = original_key
            .strip_prefix("metadata.")
            .unwrap_or(original_key);
        let mut value = None;
        let mut container = Some(metadata);
        for part in key.split('.') {
            let Some(object) = container else {
                value = None;
                break;
            };
            value = object.get(part);
            container = value.and_then(as_object);
        }
        if let Some(Json::Text(text)) = value {
            labels.insert(original_key.replace('.', "_"), text.clone());
        }
    }
    labels
}

#[path = "custom_metadata_extraction/tests.rs"]
mod tests;
