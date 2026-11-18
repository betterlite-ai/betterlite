use crate::cooldown_state::filter::HealthInputs;
use crate::deployment_identity::spec::DeploymentSpec;

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

    // _async_filter_health_check_unhealthy_deployments: whether health routing is on, its group allowlist, and the deployments the snapshot marks unhealthy
    #[must_use]
    fn health_inputs(&self) -> HealthInputs {
        HealthInputs::NONE
    }

    // async_callback_filter_deployments: each callback replaces the pool, and its failure stops the route
    fn filter_deployments(
        &self,
        deployments: Vec<DeploymentSpec>,
    ) -> Result<Vec<DeploymentSpec>, Cancelled> {
        Ok(deployments)
    }
}
