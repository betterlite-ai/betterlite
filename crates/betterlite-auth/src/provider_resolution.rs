use betterlite_models::decisions::Json;

use crate::rejection_provider::{
    DeploymentSnapshot, ProviderResolution, SnapshotError, resolve_llm_provider_for_rate_limit,
};

mod tests;

// LiteLLM's `llm_router` is None before startup builds one, and alias resolution then finds no deployment.
struct NoRouter;

impl DeploymentSnapshot for NoRouter {
    fn model_list(&self) -> Result<Option<&[Json]>, SnapshotError> {
        Ok(None)
    }
}

// rate_limiter_utils.resolve_llm_provider_for_rate_limit: native provider, then router aliases, then the fallback pair.
#[must_use]
pub fn resolve_rate_limit_provider(
    model: Option<&str>,
    native: &dyn ProviderResolution,
    router: Option<&dyn DeploymentSnapshot>,
) -> (String, String) {
    let Some(present) = router else {
        return resolve_llm_provider_for_rate_limit(model, native, &NoRouter);
    };
    resolve_llm_provider_for_rate_limit(model, native, present)
}
