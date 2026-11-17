use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::router_core::services::RouteServices;
use betterlite_router::router_predicates::params::{ParamSupport, ResolvedProvider};
use betterlite_router::static_eligibility::tokens::TokenLimits;

pub(super) struct NoServices;
impl RouteServices for NoServices {}
pub(super) struct NoLimits;
impl TokenLimits for NoLimits {
    fn max_input_tokens(&self, _spec: &DeploymentSpec, _model: &str) -> Option<usize> {
        None
    }
}
pub(super) struct NoParams;
impl ParamSupport for NoParams {
    fn is_authenticating_provider(&self, _model: &str, _custom_llm_provider: Option<&str>) -> bool {
        false
    }
    fn resolve_provider(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<ResolvedProvider> {
        None
    }
    fn supported_openai_params(
        &self,
        _model: &str,
        _custom_llm_provider: &str,
        _base_model: Option<&str>,
    ) -> Option<Vec<String>> {
        None
    }
}
