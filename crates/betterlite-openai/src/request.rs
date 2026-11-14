use secrecy::SecretString;
use serde_json::Value;

use crate::chat::{ChatRequest, RequestError};

mod tests;

// LiteLLM reads these request keys itself and never forwards them; a request destination stays B052's 400
const LITELLM_REQUEST_KEYS: &str = "api_base api_key api_version base_url caching context_window_fallback_dict extra_headers max_retries mock_delay mock_timeout num_retries stream_timeout timeout";

impl ChatRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self, RequestError> {
        let request: Self =
            serde_json::from_slice(bytes).map_err(|source| RequestError::Body { source })?;
        request.reject_unsupported()?;
        Ok(request)
    }

    // the key leaves the request here, so no body, log or Debug print of the request can carry it
    pub fn parse_with_api_key(bytes: &[u8]) -> Result<(Self, Option<SecretString>), RequestError> {
        let mut request: Self =
            serde_json::from_slice(bytes).map_err(|source| RequestError::Body { source })?;
        let key = match request.passthrough.shift_remove("api_key") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(SecretString::from(text)),
            Some(_) => return Err(RequestError::FieldType { name: "api_key" }),
        };
        request.reject_unsupported()?;
        Ok((request, key))
    }

    fn reject_unsupported(&self) -> Result<(), RequestError> {
        self.passthrough
            .keys()
            .find(|name| {
                LITELLM_REQUEST_KEYS
                    .split(' ')
                    .any(|known| known == name.as_str())
            })
            .map_or(Ok(()), |name| {
                Err(RequestError::Unsupported { name: name.clone() })
            })
    }
}
