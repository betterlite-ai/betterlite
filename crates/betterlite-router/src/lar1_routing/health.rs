use async_trait::async_trait;

use crate::deployment_catalog::types::Candidates;
use crate::router_core::error::RoutingError;

// Router.async_get_healthy_deployments: one deployment for a model id, the healthy pool for a model group
#[async_trait]
pub trait HealthyDeployments: Sync {
    async fn healthy_deployments(
        &self,
        model: &str,
        specific_deployment: bool,
    ) -> Result<Candidates, RoutingError>;
}
