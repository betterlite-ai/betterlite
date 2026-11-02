mod tests;

use axum::http::HeaderMap;

const MODEL_ID_HEADER: &str = "x-litellm-model-id";

pub(super) fn extract_model_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get(MODEL_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
