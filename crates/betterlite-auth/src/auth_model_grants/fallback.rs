use crate::auth_model_patterns::decision::ModelAccessContext;
use crate::auth_request_models::RequestedModels;
use crate::entity_model_access::error::EntityModelAccessError;
use crate::entity_model_access::resolved::{ResolvedDeps, can_key_call_resolved_model};
use crate::resolved_key::auth::UserAPIKeyAuth;

// is_model_authorized_for_token: a denial is silent, and no other failure may replace the provider error
pub async fn is_model_authorized_for_token(
    model: &str,
    token: &UserAPIKeyAuth,
    context: &ModelAccessContext<'_>,
    deps: &ResolvedDeps<'_>,
) -> bool {
    let requested = RequestedModels::One(model.to_owned());
    match can_key_call_resolved_model(&requested, token, context, deps).await {
        Ok(()) => true,
        Err(EntityModelAccessError::Denied(_)) => false,
        // fail closed: the fallback neither runs nor hides the original provider error
        Err(error) => {
            tracing::warn!(
                "Skipping fallback to model={model}: authorization lookup failed: {error}"
            );
            false
        }
    }
}

// RouterFallbackAccessCheck.__call__: enforcement off and requests without a key are not restricted
pub struct RouterFallbackAccessCheck<'check> {
    pub is_enforced: bool,
    pub token: Option<&'check UserAPIKeyAuth>,
    pub context: &'check ModelAccessContext<'check>,
    pub deps: &'check ResolvedDeps<'check>,
}

impl RouterFallbackAccessCheck<'_> {
    pub async fn call(&self, model: &str) -> bool {
        if !self.is_enforced {
            return true;
        }
        let Some(token) = self.token else {
            return true;
        };
        is_model_authorized_for_token(model, token, self.context, self.deps).await
    }
}
