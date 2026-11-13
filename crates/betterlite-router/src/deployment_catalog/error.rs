use thiserror::Error;

// The 400s of Router._common_checks_available_deployment, worded as LiteLLM words them.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CatalogLookupError {
    #[error("You passed in model={model}. There are no healthy deployments for this model")]
    NoHealthyDeployments { model: String },
    #[error(
        "You passed in model={model}. Every deployment for it is a strategy router marker (auto_router/...), which is not a callable model, and no pre-routing strategy selected a deployment for this request"
    )]
    OnlyStrategyMarkers { model: String },
    #[error(
        "Model name '{model}' matches deployments from multiple teams. Specify the deployment ID directly to disambiguate."
    )]
    AmbiguousTeams { model: String },
    // the reservation filter owns the wording of its window message
    #[error("{message}")]
    Reserved { message: String },
    #[error("the wildcard candidate {model_id:?} is not in the model list")]
    UnindexedCandidate { model_id: Option<String> },
}
