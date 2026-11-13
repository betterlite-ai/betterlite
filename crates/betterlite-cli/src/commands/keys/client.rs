use betterlite_config::sections::settings::JsonValue;
use betterlite_redact::secret::Redactor;
use http::Method;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::SecretString;

use crate::commands::Client;

use super::api::{self, ApiError, Call};

pub(in crate::commands) struct KeysClient<'run> {
    pub(in crate::commands) transport: &'run Client,
    pub(in crate::commands) base_url: &'run str,
    pub(in crate::commands) api_key: Option<&'run SecretString>,
    pub(in crate::commands) redactor: &'run Redactor,
}

pub(in crate::commands) struct JsonReply {
    pub(in crate::commands) value: JsonValue,
    pub(in crate::commands) url: String,
}

impl KeysClient<'_> {
    pub(in crate::commands) fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<Vec<u8>>,
    ) -> Result<JsonReply, ApiError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let request = Call {
            method,
            base_url: self.base_url,
            path,
            query,
            headers,
            body,
            api_key: self.api_key,
        };
        let reply = api::call(self.transport, request)?;
        let value = api::interpret(&reply, Some(self.redactor))?;
        Ok(JsonReply {
            value,
            url: reply.url,
        })
    }
}
