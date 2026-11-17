use crate::deployment_identity::spec::DeploymentSpec;
use crate::prompt_affinity::service::PromptAffinity;

// The one failure a route service reports: the caller cancelled while the deployments were filtered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the request was cancelled while the deployments were filtered")]
pub struct Cancelled;

// Router.default_cooldown_time, which get_min_cooldown reports when no deployment of the model is cooling down
const DEFAULT_MIN_COOLDOWN: f64 = 5.0;

// The dynamic state a route reads: every method is an insertion point, and a router with none answers the default.
pub trait RouteServices: Send + Sync {
    // cooldown_cache.get_min_cooldown, which the no-deployment error reports
    #[must_use]
    fn min_cooldown(&self, _model_ids: &[String]) -> f64 {
        DEFAULT_MIN_COOLDOWN
    }

    // _get_cooldown_deployments, every deployment cooling down right now
    #[must_use]
    fn cooldown_list(&self) -> Vec<String> {
        vec![]
    }

    // async_callback_filter_deployments: each callback replaces the pool, and its failure stops the route
    fn filter_deployments(
        &self,
        deployments: Vec<DeploymentSpec>,
    ) -> Result<Vec<DeploymentSpec>, Cancelled> {
        Ok(deployments)
    }

    // Router.add_optional_pre_call_checks leaves PromptCachingDeploymentCheck out unless the config names it
    #[must_use]
    fn prompt_affinity(&self) -> Option<&PromptAffinity> {
        None
    }
}
