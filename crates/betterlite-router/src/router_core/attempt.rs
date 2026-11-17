use crate::deployment_catalog::types::AliasStep;
use crate::deployment_clients::clients::CachedClient;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::strategy::Strategy;

// The deployment one route settled on, with the parts of the attempt the caller dispatches and stamps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptResult {
    pub group: String,
    pub deployment: DeploymentSpec,
    pub aliases: Vec<AliasStep>,
    // None when a specific deployment was named, since no strategy chose it
    pub strategy: Option<Strategy>,
    pub client: Option<CachedClient>,
}
