use crate::loader::EnvironmentLookup;
use crate::types::LiteLlmSettings;

pub(crate) mod dispatch;
pub(crate) mod embeddings;
pub(crate) mod images;
pub(crate) mod options;
pub(crate) mod partner_count;
pub(crate) mod partner_options;
pub(crate) mod request;
pub(crate) mod studio_chat;
pub(crate) mod thinking;

const DEFER_SETUP_VARIABLE: &str = "LITELLM_GEMINI_LIVE_DEFER_SETUP";

impl LiteLlmSettings {
    #[must_use]
    pub fn gemini_live_defer_setup(
        configured: Option<bool>,
        lookup: &dyn EnvironmentLookup,
    ) -> bool {
        match configured {
            Some(value) => value,
            // LiteLLM lowercases the value and compares it to "true" without trimming
            None => lookup
                .lookup(DEFER_SETUP_VARIABLE)
                .is_some_and(|value| value.to_lowercase() == "true"),
        }
    }
}
