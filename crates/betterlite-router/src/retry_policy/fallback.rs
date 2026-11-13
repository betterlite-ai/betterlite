use betterlite_config::sections::fallback_plans::chains::FallbackTarget;
use betterlite_config::types::RouterFallback;

// LiteLLM reads the first "*" entry with a non-empty list and returns its first target
#[must_use]
pub fn first_default_fallback(fallbacks: &[RouterFallback]) -> Option<&FallbackTarget> {
    for fallback in fallbacks {
        if let Some(first) = fallback.get("*").and_then(|targets| targets.first()) {
            return Some(first);
        }
    }
    None
}
