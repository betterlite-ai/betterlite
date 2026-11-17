use crate::deployment_catalog::types::Authority;
use crate::deployment_clients::clients::{CachedClients, ClientKind};
use crate::router_core::services::RouteServices;
use crate::router_predicates::params::ParamSupport;
use crate::static_eligibility::request::RequestFacts;
use crate::static_eligibility::tokens::TokenLimits;

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

// The router settings one route applies, resolved by betterlite-config.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoutePolicy {
    pub enable_pre_call_checks: bool,
    pub drop_params: bool,
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
    pub policy: RoutePolicy,
    pub clients: Option<ClientAccess<'route>>,
}
