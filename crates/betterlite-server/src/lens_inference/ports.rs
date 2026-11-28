use chrono::{DateTime, Utc};

use crate::lens_inference::quote::Deployment;

// the clock analyze reads for the budget month and reserve reads again for the lease deadline
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

// llm_router: configured() is false when the proxy has no analysis router, as litellm's llm_router may be None
pub trait ModelCatalog: Send + Sync {
    fn configured(&self) -> bool;

    fn deployments(&self, model: &str, team_id: Option<&str>) -> Vec<Deployment>;
}
