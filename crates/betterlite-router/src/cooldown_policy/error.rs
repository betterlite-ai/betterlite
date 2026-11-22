use thiserror::Error;

use crate::model_catalog_lookup::error::LookupError;
use crate::router_observations::counter::CounterError;
use crate::routing_value_store::StoreError;

#[derive(Debug, Error)]
pub enum CooldownPolicyError {
    #[error("the deployment lookup failed")]
    Lookup {
        #[from]
        source: LookupError,
    },
    #[error("counting a failure for {model_id} failed")]
    Counter {
        model_id: String,
        #[source]
        source: CounterError,
    },
    #[error("the cooldown entry for {model_id} could not be stored")]
    Cooldown {
        model_id: String,
        #[source]
        source: StoreError,
    },
    #[error("reading the cooldown entries failed")]
    CooldownRead {
        #[from]
        source: StoreError,
    },
    #[error("the {field} of deployment {model_id} is not the type LiteLLM reads")]
    DeploymentFact {
        model_id: String,
        field: &'static str,
    },
}
