use betterlite_openai::openai_video_identifiers::ids::decode_video_id_with_provider;
use betterlite_prompts::arguments::json::{Json, Object};

use super::error::VeoOperationError;

pub struct FetchRequest {
    pub url: String,
    pub params: Object,
}

// transform_video_status_retrieve_request: content and edit prefetch delegate to this same polling request
pub fn fetch_operation_request(
    video_id: &str,
    api_base: &str,
) -> Result<FetchRequest, VeoOperationError> {
    let operation = decode_video_id_with_provider(video_id);
    let model = operation.video_id.split('/').nth(7).unwrap_or_default();
    if model.is_empty() {
        return Err(VeoOperationError::InvalidOperation {
            operation: operation.video_id,
        });
    }
    let mut params = Object::new();
    params.insert(
        "operationName".to_owned(),
        Json::Text(operation.video_id.clone()),
    );
    Ok(FetchRequest {
        url: format!(
            "{}/{model}:fetchPredictOperation",
            api_base.trim_end_matches('/')
        ),
        params,
    })
}
