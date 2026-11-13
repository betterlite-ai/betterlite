mod tests;

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::decisions::transformation::provider_config::DecisionsProviderConfig;

const DEFAULT_BASE: &str = "https://api.cloudflare.com/client/v4/accounts";
const LEGACY_PATH: &str = "/ai/run";
const OPENAI_PATH: &str = "/ai/v1";
const CANONICAL_PREFIX: &str = "@cf/";
const CANONICAL_PROVIDER: &str = "@cf/cloudflare/";
const MISSING_BASE_MESSAGE: &str =
    "Missing CLOUDFLARE_ACCOUNT_ID - set CLOUDFLARE_ACCOUNT_ID or pass api_base explicitly";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CloudflareDecisionsConfig {
    account_id: Option<String>,
}

impl CloudflareDecisionsConfig {
    #[must_use]
    pub fn new(account_id: Option<String>) -> Self {
        let trimmed = account_id.map(|id| id.trim().to_owned());
        Self {
            account_id: trimmed.filter(|id| !id.is_empty()),
        }
    }
}

// a borrowed key map keeps every key, so a null answers value still marks the key as present
#[derive(Deserialize)]
#[serde(transparent)]
struct ReplyEnvelope<'call> {
    #[serde(borrow)]
    fields: BTreeMap<String, &'call RawValue>,
}

impl DecisionsProviderConfig for CloudflareDecisionsConfig {
    const API_KEY_ENV: &'static [&'static str] = &["CLOUDFLARE_API_KEY"];
    const API_BASE_ENV: &'static [&'static str] = &["CLOUDFLARE_API_BASE"];

    fn get_default_api_base(&self) -> Option<String> {
        self.account_id
            .as_ref()
            .map(|id| format!("{DEFAULT_BASE}/{id}{LEGACY_PATH}"))
    }

    fn missing_api_base_message(&self, _custom_llm_provider: &str) -> String {
        MISSING_BASE_MESSAGE.to_owned()
    }

    fn canonical_model(&self, model: &str) -> String {
        if model.starts_with(CANONICAL_PREFIX) {
            return model.to_owned();
        }
        format!("{CANONICAL_PROVIDER}{model}")
    }

    fn request_model(&self, model: &str) -> String {
        model
            .rsplit_once('/')
            .map_or(model, |(_, name)| name)
            .to_owned()
    }

    fn get_complete_url(&self, api_base: &str, model: &str) -> String {
        let base = api_base.trim_end_matches('/');
        if let Some(root) = base.strip_suffix(OPENAI_PATH) {
            return format!("{root}{LEGACY_PATH}/{model}");
        }
        if base.ends_with(LEGACY_PATH) {
            return format!("{base}/{model}");
        }
        format!("{base}{LEGACY_PATH}/{model}")
    }

    fn unwrap_response<'call>(&self, payload: &'call [u8]) -> &'call [u8] {
        let Ok(envelope) = serde_json::from_slice::<ReplyEnvelope<'call>>(payload) else {
            return payload;
        };
        if envelope.fields.contains_key("answers") {
            return payload;
        }
        let Some(result) = envelope.fields.get("result") else {
            return payload;
        };
        // a JSON object's raw text begins with '{'; arrays, strings, numbers and null do not
        if !result.get().starts_with('{') {
            return payload;
        }
        result.get().as_bytes()
    }
}
