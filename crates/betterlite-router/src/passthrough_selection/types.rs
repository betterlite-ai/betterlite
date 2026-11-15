use std::collections::BTreeSet;

use crate::deployment_catalog::lookup::CatalogResolver;
use crate::deployment_catalog::types::Authority;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::static_eligibility::filter::EligibilityServices;
use crate::static_eligibility::request::RequestFacts;
use crate::strategy_dispatch::groups::RoutingGroups;
use crate::strategy_dispatch::selector::SelectorId;
use crate::strategy_dispatch::types::{RequestBindings, Resolution, StrategyOverride};

// One pass-through request: the model to select, the caller's authority and the strategy it asked for.
pub struct SelectionRequest<'request> {
    pub model: &'request str,
    pub specific_deployment: bool,
    pub authority: Authority<'request>,
    pub facts: &'request RequestFacts,
    pub strategy: StrategyOverride<'request>,
    pub bindings: Option<&'request mut RequestBindings>,
}

// The router services a selection reads; each one belongs to the service that built it.
pub struct SelectionServices<'services, 'catalog> {
    pub catalog: &'services CatalogResolver<'catalog>,
    pub eligibility: &'services EligibilityServices<'services>,
    pub groups: &'services RoutingGroups,
    pub shadowing: &'services BTreeSet<String>,
    pub environment: Option<&'services str>,
    pub enable_pre_call_checks: bool,
    pub drop_params: bool,
}

// The chosen deployment, the strategy decision behind it and the selector its pre-call check runs for.
pub struct Selected {
    pub deployment: DeploymentSpec,
    pub resolution: Resolution,
    pub override_pre_call: Option<SelectorId>,
}
