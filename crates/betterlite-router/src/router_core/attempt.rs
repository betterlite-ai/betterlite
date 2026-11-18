use crate::deployment_catalog::types::AliasStep;
use crate::deployment_clients::clients::CachedClient;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::least_busy::guard::InFlightCall;
use crate::strategy::Strategy;
use crate::strategy_dispatch::selector::SelectorId;
use crate::strategy_dispatch::types::RequestBindings;

// The deployment one route settled on, with the parts of the attempt the caller dispatches and stamps.
#[derive(Debug)]
pub struct AttemptResult {
    pub group: String,
    pub deployment: DeploymentSpec,
    pub aliases: Vec<AliasStep>,
    // None when a specific deployment was named, since no strategy chose it
    pub strategy: Option<Strategy>,
    pub client: Option<CachedClient>,
    // The override selector whose pre-call check runs for this request, when the request named its own strategy
    pub override_pre_call: Option<SelectorId>,
    // The selector callbacks the routing resolution bound to this request
    pub bindings: RequestBindings,
    // The least-busy count this attempt started, absent on a router that keeps no counters; the caller releases it once
    pub least_busy: Option<InFlightCall>,
}
