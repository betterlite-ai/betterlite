use secrecy::SecretString;
use serde_json::Value;

use crate::chat::{ChatRequest, RequestError};

mod tests;

// LiteLLM reads these request keys itself and never forwards them; a request destination stays B052's 400
const LITELLM_REQUEST_KEYS: &str = "api_base api_version base_url caching context_window_fallback_dict extra_headers max_retries mock_delay mock_timeout num_retries stream_timeout timeout";

impl ChatRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self, RequestError> {
        let request: Self =
            serde_json::from_slice(bytes).map_err(|source| RequestError::Body { source })?;
        request.client_api_key()?;
        if let Some(name) = request.passthrough.keys().find(|name| {
            LITELLM_REQUEST_KEYS
                .split(' ')
                .any(|known| known == name.as_str())
        }) {
            return Err(RequestError::Unsupported { name: name.clone() });
        }
        Ok(request)
    }

    // the request-level key LiteLLM reads itself and hands to the clientside credential handler
    pub fn client_api_key(&self) -> Result<Option<SecretString>, RequestError> {
        match self.passthrough.get("api_key") {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(SecretString::from(text.clone()))),
            Some(_) => Err(RequestError::FieldType { name: "api_key" }),
        }
    }

    // moves the key out of the passthrough bucket, so the body forwarded to a provider never carries it
    pub fn take_client_api_key(&mut self) -> Result<Option<SecretString>, RequestError> {
        let key = self.client_api_key()?;
        self.passthrough.shift_remove("api_key");
        Ok(key)
    }
}
