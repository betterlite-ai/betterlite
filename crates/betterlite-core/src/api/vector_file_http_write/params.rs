use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_file_contracts::records::VectorStoreFileUpdateRequest;
use indexmap::IndexMap;

use super::error::FileWriteError;

// dict(update_request), then extra_body's keys last: the provider's update transform reads this merged map
pub fn merged_request(
    update_request: &VectorStoreFileUpdateRequest,
    extra_body: Option<&IndexMap<String, Json>>,
) -> Result<IndexMap<String, Json>, FileWriteError> {
    let mut payload = IndexMap::new();
    if let Some(attributes) = &update_request.attributes {
        let encoded = serde_json::to_value(attributes).map_err(FileWriteError::Encode)?;
        let typed = serde_json::from_value(encoded).map_err(FileWriteError::Encode)?;
        payload.insert(String::from("attributes"), typed);
    }
    for (name, value) in &update_request.extra {
        let typed = serde_json::from_value(value.clone()).map_err(FileWriteError::Encode)?;
        payload.insert(name.clone(), typed);
    }
    for (name, value) in extra_body.into_iter().flatten() {
        payload.insert(name.clone(), value.clone());
    }
    Ok(payload)
}
