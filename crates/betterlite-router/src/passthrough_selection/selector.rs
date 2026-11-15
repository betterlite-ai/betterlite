use crate::deployment_identity::spec::DeploymentSpec;
use crate::static_eligibility::request::RequestFacts;
use crate::strategy_dispatch::selection::SelectorCall;
use crate::strategy_dispatch::selector::SelectorId;

// The selector strategies keep their own state; the selection hands one call over and takes an index back.
pub trait SelectorCaller {
    fn pick(
        &mut self,
        call: SelectorCall,
        selector: SelectorId,
        deployments: &[DeploymentSpec],
        facts: &RequestFacts,
    ) -> Option<usize>;
}
