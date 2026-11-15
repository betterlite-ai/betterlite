use crate::deployment_catalog::error::CatalogLookupError;
use crate::static_eligibility::error::EligibilityError;
use crate::strategy_dispatch::error::StrategyDispatchError;
use crate::strategy_dispatch::selector::SelectorId;

// Every failure of async_get_available_deployment_for_pass_through: the catalog 400s, the paused 503 and its no-deployments error.
#[derive(Debug, thiserror::Error)]
pub enum PassthroughSelectionError {
    #[error(transparent)]
    Catalog(#[from] CatalogLookupError),
    #[error(transparent)]
    Eligibility(#[from] EligibilityError),
    #[error(transparent)]
    Strategy(#[from] StrategyDispatchError),
    #[error(
        "Model {model} has no deployments configured with use_in_pass_through=True. Please add use_in_pass_through: true to the deployment configuration"
    )]
    NotConfigured { model: String },
    #[error(
        "Deployment {deployment_id} does not support pass-through endpoint (use_in_pass_through=False)"
    )]
    DeploymentNotConfigured { deployment_id: String },
    #[error("no pass-through deployment is available for model {model}")]
    NoDeployment { model: String },
    #[error(
        "selector {selector:?} chose deployment {index}, but only {available} pass-through deployments exist"
    )]
    SelectorOutOfRange {
        selector: SelectorId,
        index: usize,
        available: usize,
    },
    #[error("deployment {deployment_id}: litellm_params.{field} must be {expected}")]
    InvalidDeploymentField {
        deployment_id: String,
        field: &'static str,
        expected: &'static str,
    },
}
