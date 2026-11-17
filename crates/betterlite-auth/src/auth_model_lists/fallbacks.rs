use betterlite_config::sections::fallback_plans::Settings;
use betterlite_config::sections::fallback_plans::chains::FallbackChainEntry;
pub const GENERAL_FALLBACK_TYPE: &str = "general";
pub const CONTEXT_WINDOW_FALLBACK_TYPE: &str = "context_window";
pub const CONTENT_POLICY_FALLBACK_TYPE: &str = "content_policy";
// B309's get_fallback_model_group, injected so a listing declares no permission to call a fallback.
#[derive(Debug, thiserror::Error)]
pub enum FallbackError {
    #[error("the fallback plan could not be resolved")]
    Unresolvable,
}
pub trait FallbackGroups: Send + Sync {
    fn fallback_group(
        &self,
        chain: &[FallbackChainEntry],
        model: &str,
    ) -> Result<Option<Vec<String>>, FallbackError>;
}
// get_all_fallbacks: the names of one fallback category; a listing never grants the models it names.
#[must_use]
pub fn get_all_fallbacks(
    model: &str,
    fallback_type: &str,
    settings: Option<&Settings>,
    groups: &dyn FallbackGroups,
) -> Vec<String> {
    let Some(configured) = settings else {
        return Vec::new();
    };
    let chain = match fallback_type {
        GENERAL_FALLBACK_TYPE => &configured.fallbacks,
        CONTEXT_WINDOW_FALLBACK_TYPE => &configured.context_window_fallbacks,
        CONTENT_POLICY_FALLBACK_TYPE => &configured.content_policy_fallbacks,
        unknown => {
            tracing::warn!("Unknown fallback_type: {unknown}");
            return Vec::new();
        }
    };
    if chain.is_empty() {
        return Vec::new();
    }
    match groups.fallback_group(chain, model) {
        Ok(Some(fallbacks)) => fallbacks,
        Ok(None) => Vec::new(),
        Err(error) => {
            tracing::error!("Error getting fallbacks for model {model}: {error}");
            Vec::new()
        }
    }
}
