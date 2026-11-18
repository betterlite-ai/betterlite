use thiserror::Error;

use crate::deployment_catalog::error::CatalogLookupError;
use crate::prompt_affinity::service::PromptAffinityError;
use crate::router_core::no_deployment::NoDeployment;
use crate::router_core::services::Cancelled;
use crate::simple_shuffle::error::ShuffleError;
use crate::static_eligibility::error::EligibilityError;

// Every way the route pipeline stops before it hands a deployment to the caller.
#[derive(Debug, Error)]
pub enum RoutingError {
    #[error(transparent)]
    Catalog(#[from] CatalogLookupError),
    #[error(transparent)]
    Eligibility(#[from] EligibilityError),
    #[error(transparent)]
    NoDeployment(#[from] NoDeployment),
    // a strategy or stage another task owns answers this until that task lands
    #[error("the {entry} routing entry is not built")]
    NotBuilt { entry: &'static str },
    // a caller cancelled the request, which LiteLLM lets through the callback filter untouched
    #[error("the request was cancelled before a deployment was selected")]
    Cancelled,
    // the prompt caching check raises a prefix it cannot read, as async_callback_filter_deployments does
    #[error(transparent)]
    PromptAffinity(#[from] PromptAffinityError),
    #[error("the deployments cannot be weighed for a simple-shuffle pick")]
    Shuffle {
        #[from]
        source: ShuffleError,
    },
    // _metric_weight's TypeError: a litellm_params weight that is not a number names its field
    #[error("Deployment {field} must be numeric")]
    Metric { field: &'static str },
}

impl RoutingError {
    // the entry an unbuilt strategy or stage names, which the stub tests assert
    #[must_use]
    pub const fn unbuilt_entry(&self) -> Option<&'static str> {
        match self {
            Self::NotBuilt { entry } => Some(entry),
            Self::Catalog(_)
            | Self::Eligibility(_)
            | Self::NoDeployment(_)
            | Self::Cancelled
            | Self::PromptAffinity(_)
            | Self::Shuffle { .. }
            | Self::Metric { .. } => None,
        }
    }
}

impl From<Cancelled> for RoutingError {
    fn from(_cancelled: Cancelled) -> Self {
        Self::Cancelled
    }
}
