use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;

use crate::context::{self, RouteStage};
use crate::deployment_catalog::types::{AliasStep, Authority};
use crate::deployment_clients::clients::{CachedClients, ClientKind};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::candidates::CandidateSet;
use crate::router_core::error::RoutingError;
use crate::router_core::no_deployment::NoDeployment;
use crate::router_core::services::RouteServices;
use crate::router_predicates::params::ParamSupport;
use crate::static_eligibility::request::RequestFacts;
use crate::static_eligibility::tokens::TokenLimits;
use crate::strategy_dispatch::selector::SelectorId;
use crate::strategy_dispatch::types::{RequestBindings, Resolution};

// What one request asks the router for, typed: no request dict is read or written.
pub struct RouteRequest<'request> {
    pub model: &'request str,
    pub facts: &'request RequestFacts,
    pub authority: Authority<'request>,
    pub environment: Option<&'request str>,
    pub specific_deployment: bool,
    pub affinity_pinned: bool,
    pub excluded_deployment_ids: &'request [String],
    pub retry_skipped_deployment_ids: &'request [String],
}

// The prompt and request arguments the prompt caching check hashes, absent on a route that carries none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PromptInputs<'request> {
    pub messages: Option<&'request [Message]>,
    pub request_kwargs: Option<&'request Object>,
}

// The router settings one route applies, resolved by betterlite-config, with the request inputs its checks read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoutePolicy<'request> {
    pub enable_pre_call_checks: bool,
    pub drop_params: bool,
    pub prompt: PromptInputs<'request>,
}

// The client store a route acquires the selected deployment's client from; absent means the attempt carries none.
#[derive(Clone, Copy)]
pub struct ClientAccess<'access> {
    pub clients: &'access CachedClients,
    pub kind: ClientKind,
    pub stream: bool,
}

// Everything one route reads: the request, the services it consults and the settings it applies.
pub struct RouteContext<'route> {
    pub request: &'route RouteRequest<'route>,
    pub services: &'route dyn RouteServices,
    pub limits: &'route (dyn TokenLimits + Sync),
    pub params: &'route (dyn ParamSupport + Sync),
    pub policy: RoutePolicy<'route>,
    pub clients: Option<ClientAccess<'route>>,
}

// What one request resolved to, before any filter touched it: the catalog's match and B395's routing decision.
pub(crate) struct ResolvedRoute {
    pub(crate) group: String,
    pub(crate) aliases: Vec<AliasStep>,
    pub(crate) model_ids: Vec<String>,
    pub(crate) resolution: Resolution,
    pub(crate) override_pre_call: Option<SelectorId>,
    pub(crate) bindings: RequestBindings,
}

// One deployment through a stage that may drop it: what survived, or none, which the caller answers as no deployment.
pub(crate) async fn stage_single(
    route: &RouteContext<'_>,
    stage: RouteStage,
    deployment: DeploymentSpec,
) -> Result<Option<DeploymentSpec>, RoutingError> {
    let candidates = CandidateSet {
        deployments: vec![deployment],
    };
    Ok(context::run(stage, candidates, route)
        .await?
        .deployments
        .pop())
}

// async_raise_no_deployment_exception, in the fields the proxy and the caller read back
pub(crate) fn no_deployment_error(
    context: &RouteContext<'_>,
    resolved: &ResolvedRoute,
) -> RoutingError {
    RoutingError::NoDeployment(NoDeployment::new(
        resolved.group.clone(),
        context.services.min_cooldown(&resolved.model_ids),
        context.policy.enable_pre_call_checks,
        context.services.cooldown_list(),
        resolved.model_ids.clone(),
    ))
}
