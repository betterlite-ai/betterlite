use std::future::{Future, ready};

use betterlite_core::api::copilot_chat::credentials::CopilotCredentials;
use betterlite_openai::copilot::api_key::error::ApiKeyError;
use secrecy::SecretString;

// A cached Copilot record: its token may be absent, and it may carry the session host.
pub(super) struct StoredKey {
    token: Option<SecretString>,
    base: Option<String>,
}

impl StoredKey {
    pub(super) fn new(token: Option<&str>, base: Option<&str>) -> Self {
        Self {
            token: token.map(|text| SecretString::from(text.to_owned())),
            base: base.map(str::to_owned),
        }
    }
}

impl CopilotCredentials for StoredKey {
    fn api_base(&mut self) -> impl Future<Output = Option<String>> + Send {
        ready(self.base.clone())
    }

    fn api_key(
        &mut self,
    ) -> impl Future<Output = Result<Option<SecretString>, ApiKeyError>> + Send {
        ready(Ok(self.token.clone()))
    }
}
