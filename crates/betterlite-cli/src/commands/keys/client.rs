use betterlite_config::sections::settings::JsonValue;
use betterlite_redact::secret::Redactor;
use http::Method;
use secrecy::SecretString;

use crate::commands::Client;

use super::api::{self, ApiError, Call};

pub(super) struct KeysClient<'run> {
    pub(super) transport: &'run Client,
    pub(super) base_url: &'run str,
    pub(super) api_key: Option<&'run SecretString>,
    pub(super) redactor: &'run Redactor,
}

pub(super) struct JsonReply {
    pub(super) value: JsonValue,
    pub(super) url: String,
}

impl KeysClient<'_> {
    pub(super) fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<Vec<u8>>,
    ) -> Result<JsonReply, ApiError> {
        let request = Call {
            method,
            base_url: self.base_url,
            path,
            query,
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
