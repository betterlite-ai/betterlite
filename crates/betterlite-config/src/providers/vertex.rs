use crate::loader::EnvironmentLookup;

pub(crate) mod dispatch;
pub(crate) mod embeddings;
pub(crate) mod images;
pub(crate) mod thinking;

const DEFER_SETUP_VARIABLE: &str = "LITELLM_GEMINI_LIVE_DEFER_SETUP";

pub(crate) fn gemini_live_defer_setup(
    configured: Option<bool>,
    lookup: &dyn EnvironmentLookup,
) -> bool {
    // LiteLLM lowercases the value and compares it to "true" without trimming
    configured.unwrap_or_else(|| {
        lookup
            .lookup(DEFER_SETUP_VARIABLE)
            .is_some_and(|value| value.to_lowercase() == "true")
    })
}
