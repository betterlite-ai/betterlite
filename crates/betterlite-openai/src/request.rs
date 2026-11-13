use crate::chat::{ChatRequest, RequestError};

mod tests;

// LiteLLM reads these request keys itself and never forwards them, and betterlite does not act on them yet
const LITELLM_REQUEST_KEYS: &str = "api_base api_key api_version base_url caching context_window_fallback_dict extra_headers max_retries mock_delay mock_timeout num_retries stream_timeout timeout";

impl ChatRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self, RequestError> {
        let request: Self =
            serde_json::from_slice(bytes).map_err(|source| RequestError::Body { source })?;
        if let Some(name) = request.passthrough.keys().find(|name| {
            LITELLM_REQUEST_KEYS
                .split(' ')
                .any(|known| known == name.as_str())
        }) {
            return Err(RequestError::Unsupported { name: name.clone() });
        }
        Ok(request)
    }
}
