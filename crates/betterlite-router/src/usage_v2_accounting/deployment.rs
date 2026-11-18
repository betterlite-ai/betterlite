use crate::router_observations::usage::DeploymentUsage;

// the deployment values the v2 pre-call reads: model_info.id, litellm_params.model, the rpm limits and num_retries
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeploymentFacts<'deployment> {
    pub model_id: &'deployment str,
    pub model: &'deployment str,
    pub limits: DeploymentUsage,
    pub num_retries: Option<i64>,
}
