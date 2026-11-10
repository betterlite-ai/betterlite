use crate::identity::declared::authenticating_provider;

use super::{ProviderIdentity, ResolutionContext, ResolutionRequest, resolve};

// Ports inferred_provider (lines 146-155): the declaration wins for OAuth providers, and every failure is None.
#[must_use]
pub fn inferred_provider(model: &str, context: &ResolutionContext<'_>) -> Option<ProviderIdentity> {
    if model.is_empty() {
        return None;
    }
    if let Some(declared) = authenticating_provider(Some(model), None) {
        return Some(ProviderIdentity::Known(declared));
    }
    let request = ResolutionRequest {
        model,
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    };
    resolve(&request, context)
        .ok()
        .map(|resolved| resolved.provider)
}

mod tests;
