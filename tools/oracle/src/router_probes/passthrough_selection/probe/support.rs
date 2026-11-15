use betterlite_router::deployment_catalog::error::CatalogLookupError;
use betterlite_router::deployment_catalog::types::CandidateView;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::passthrough_selection::selector::SelectorCaller;
use betterlite_router::router_predicates::params::{ParamSupport, ResolvedProvider};
use betterlite_router::static_eligibility::request::RequestFacts;
use betterlite_router::static_eligibility::tokens::TokenLimits;
use betterlite_router::strategy_dispatch::selection::SelectorCall;
use betterlite_router::strategy_dispatch::selector::SelectorId;

// The probe router has no access groups, reserved models, default fallbacks or provider inference.
pub(super) struct NoPolicy;

impl CandidateView for NoPolicy {
    fn routing_group(&self, _: &str, _: Option<&str>) -> Option<Vec<DeploymentSpec>> {
        None
    }

    fn filter_access_groups(
        &self,
        _: &str,
        deployments: Vec<DeploymentSpec>,
        _: Option<&str>,
    ) -> Vec<DeploymentSpec> {
        deployments
    }

    fn filter_reserved(
        &self,
        deployments: Vec<DeploymentSpec>,
        _: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, CatalogLookupError> {
        Ok(deployments)
    }

    fn default_fallback(&self) -> Option<String> {
        None
    }

    fn inferred_provider(&self, _: &str) -> Option<String> {
        None
    }
}

// No case sets max_input_tokens, so the token count never enters the eligibility filters.
pub(super) struct NoLimits;

impl TokenLimits for NoLimits {
    fn max_input_tokens(&self, _: &DeploymentSpec, _: &str) -> Option<usize> {
        None
    }
}

pub(super) struct ProbeSupport;

impl ParamSupport for ProbeSupport {
    fn is_authenticating_provider(&self, _: &str, _: Option<&str>) -> bool {
        false
    }

    fn resolve_provider(&self, model: &str, _: Option<&str>) -> Option<ResolvedProvider> {
        match model.split_once('/') {
            Some((provider, bare)) => Some(ResolvedProvider {
                model: bare.to_owned(),
                custom_llm_provider: provider.to_owned(),
            }),
            None => Some(ResolvedProvider {
                model: model.to_owned(),
                custom_llm_provider: "openai".to_owned(),
            }),
        }
    }

    fn supported_openai_params(
        &self,
        model: &str,
        custom_llm_provider: &str,
        _: Option<&str>,
    ) -> Option<Vec<String>> {
        if custom_llm_provider != "openai" {
            return None;
        }
        // the two OpenAI models whose config leaves response_format out
        match model {
            "gpt-4" | "gpt-3.5-turbo-16k" => Some(Vec::new()),
            _ => Some(vec!["response_format".to_owned()]),
        }
    }
}

// Neither probed strategy calls a selector, so a call here means the probe was driven off its cases.
pub(super) struct NoSelector;

impl SelectorCaller for NoSelector {
    fn pick(
        &mut self,
        _: SelectorCall,
        _: SelectorId,
        _: &[DeploymentSpec],
        _: &RequestFacts,
    ) -> Option<usize> {
        None
    }
}
