use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use serde_json::Number;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_catalog::error::CatalogLookupError;
use crate::deployment_catalog::lookup::CatalogResolver;
use crate::deployment_catalog::types::{Authority, CandidateView};
use crate::deployment_clients::clients::{CachedClients, ClientClock, ClientKind};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::attempt::AttemptResult;
use crate::router_core::error::RoutingError;
use crate::router_core::headers::Response;
use crate::router_core::no_deployment::NoDeployment;
use crate::router_core::pipeline::{
    HealthyDeployments, StrategyRoute, healthy_deployments, prepare, resolve_catalog, route,
};
use crate::router_core::route::{ClientAccess, RouteContext, RoutePolicy, RouteRequest};
use crate::router_core::services::{Cancelled, RouteServices};
use crate::router_predicates::params::{ParamSupport, ResolvedProvider};
use crate::static_eligibility::request::RequestFacts;
use crate::static_eligibility::tokens::TokenLimits;
use crate::strategy::StrategyContext;
use crate::strategy_dispatch::groups::{RoutingGroups, RoutingScope};
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::{Resolution, StrategyOverride};

// A router that keeps no dynamic state: every insertion point answers its default.
pub(crate) struct NoServices;

impl RouteServices for NoServices {}

// A model registry that declares no context window.
pub(crate) struct NoLimits;

impl TokenLimits for NoLimits {
    fn max_input_tokens(&self, _spec: &DeploymentSpec, _model: &str) -> Option<usize> {
        None
    }
}

// A model registry that declares a context window per deployment id.
pub(crate) struct Windows {
    pub(crate) by_id: BTreeMap<String, usize>,
}

impl TokenLimits for Windows {
    fn max_input_tokens(&self, spec: &DeploymentSpec, _model: &str) -> Option<usize> {
        self.by_id.get(spec.id()?).copied()
    }
}

// A provider registry with one entry: the unknown arm knows no provider, the openai arm declares one param,
// and the mini arm declares response_format only for the models whose name carries it.
#[derive(Clone, Copy)]
pub(crate) enum Providers {
    Unknown,
    OpenAi,
    Mini,
}

impl ParamSupport for Providers {
    fn is_authenticating_provider(&self, _model: &str, _custom_llm_provider: Option<&str>) -> bool {
        false
    }

    fn resolve_provider(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<ResolvedProvider> {
        match self {
            Self::Unknown => None,
            Self::OpenAi | Self::Mini => Some(ResolvedProvider {
                model: model.to_owned(),
                custom_llm_provider: "openai".to_owned(),
            }),
        }
    }

    fn supported_openai_params(
        &self,
        model: &str,
        _custom_llm_provider: &str,
        _base_model: Option<&str>,
    ) -> Option<Vec<String>> {
        match self {
            Self::Unknown => None,
            Self::OpenAi => Some(vec![String::from("temperature")]),
            Self::Mini if model.contains("mini") => Some(vec![
                String::from("temperature"),
                String::from("response_format"),
            ]),
            Self::Mini => Some(vec![String::from("temperature")]),
        }
    }
}

// The pre-call settings a route test asks for: its limits, its providers, and whether the router checks them.
pub(crate) struct Checked<'checks> {
    pub(crate) limits: &'checks (dyn TokenLimits + Sync),
    pub(crate) params: &'checks (dyn ParamSupport + Sync),
    pub(crate) pre_call: bool,
}

// A request that names one model and asks for nothing else.
#[must_use]
pub(crate) fn route_request<'request>(
    model: &'request str,
    facts: &'request RequestFacts,
) -> RouteRequest<'request> {
    RouteRequest {
        model,
        facts,
        authority: Authority::default(),
        environment: None,
        specific_deployment: false,
        affinity_pinned: false,
        excluded_deployment_ids: &[],
        retry_skipped_deployment_ids: &[],
        messages: None,
        request_kwargs: None,
    }
}

// A route over the given request and stateless services.
#[must_use]
pub(crate) fn route_context<'route>(
    request: &'route RouteRequest<'route>,
    services: &'route dyn RouteServices,
    limits: &'route (dyn TokenLimits + Sync),
    params: &'route (dyn ParamSupport + Sync),
) -> RouteContext<'route> {
    RouteContext {
        request,
        services,
        limits,
        params,
        policy: RoutePolicy::default(),
        clients: None,
    }
}

// A strategy call over the given pool and selector state.
#[must_use]
pub(crate) fn strategy_context<'context>(
    deployments: &'context [DeploymentSpec],
    route: &'context RouteContext<'context>,
    dispatch: &'context mut StrategyDispatch,
    resolution: &'context Resolution,
) -> StrategyContext<'context> {
    StrategyContext {
        deployments,
        route,
        dispatch,
        resolution,
    }
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn number(value: u64) -> Json {
    Json::Number(Number::from(value))
}

pub(super) fn object(entries: &[(&str, Json)]) -> JsonObject {
    let mut map = JsonObject::new();
    for (key, value) in entries {
        map.insert((*key).to_owned(), value.clone());
    }
    map
}

pub(super) fn with_info(
    group: &str,
    id: &str,
    params: &[(&str, Json)],
    info: &[(&str, Json)],
) -> DeploymentSpec {
    let mut litellm = object(&[("model", text(&format!("openai/{id}")))]);
    for (key, value) in params {
        litellm.insert((*key).to_owned(), value.clone());
    }
    let mut model_info = object(&[("id", text(id))]);
    for (key, value) in info {
        model_info.insert((*key).to_owned(), value.clone());
    }
    DeploymentSpec::new(
        group.to_owned(),
        litellm,
        model_info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

pub(super) fn deployment(group: &str, id: &str, params: &[(&str, Json)]) -> DeploymentSpec {
    with_info(group, id, params, &[])
}

pub(super) fn group_and_ids(deployments: &[(&str, &str)]) -> Vec<DeploymentSpec> {
    deployments
        .iter()
        .map(|(group, id)| deployment(group, id, &[]))
        .collect()
}

// A weight that decides the pick on its own, since the deployments beside it weigh nothing
pub(super) fn weighted(group: &str, id: &str, weight: u64) -> DeploymentSpec {
    deployment(group, id, &[("weight", number(weight))])
}

// A deployment an admin paused: no route serves it, and the healthy view drops it
pub(super) fn paused(group: &str, id: &str, params: &[(&str, Json)]) -> DeploymentSpec {
    with_info(group, id, params, &[("blocked", Json::Bool(true))])
}

fn catalog_of(deployments: Vec<DeploymentSpec>) -> DeploymentCatalog {
    let mut catalog = DeploymentCatalog::default();
    for deployment in deployments {
        catalog.model_names.insert(deployment.model_name.clone());
        catalog.index_patterns(&deployment).unwrap();
        catalog.push(deployment);
    }
    catalog
}

// The policy layers a router without teams answers with: every deployment the catalog holds is reachable.
struct OpenView;

impl CandidateView for OpenView {
    fn routing_group(&self, _model: &str, _team: Option<&str>) -> Option<Vec<DeploymentSpec>> {
        None
    }

    fn filter_access_groups(
        &self,
        _model: &str,
        deployments: Vec<DeploymentSpec>,
        _team: Option<&str>,
    ) -> Vec<DeploymentSpec> {
        deployments
    }

    fn filter_reserved(
        &self,
        deployments: Vec<DeploymentSpec>,
        _team: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, CatalogLookupError> {
        Ok(deployments)
    }

    fn default_fallback(&self) -> Option<String> {
        None
    }

    fn inferred_provider(&self, _model: &str) -> Option<String> {
        None
    }
}

pub(super) struct Fixture {
    catalog: DeploymentCatalog,
    group_alias: BTreeMap<String, String>,
    model_alias: BTreeMap<String, String>,
    view: OpenView,
}

impl Fixture {
    pub(super) fn plain(deployments: Vec<DeploymentSpec>) -> Self {
        Self {
            catalog: catalog_of(deployments),
            group_alias: BTreeMap::new(),
            model_alias: BTreeMap::new(),
            view: OpenView,
        }
    }

    pub(super) fn aliased(
        deployments: Vec<DeploymentSpec>,
        group: &[(&str, &str)],
        model: &[(&str, &str)],
    ) -> Self {
        let mut fixture = Self::plain(deployments);
        for (from, to) in group {
            fixture
                .group_alias
                .insert((*from).to_owned(), (*to).to_owned());
        }
        for (from, to) in model {
            fixture
                .model_alias
                .insert((*from).to_owned(), (*to).to_owned());
        }
        fixture
    }

    pub(super) fn resolver(&self) -> CatalogResolver<'_> {
        let Self {
            catalog,
            group_alias,
            model_alias,
            view,
        } = self;
        CatalogResolver {
            catalog,
            model_group_alias: group_alias,
            model_alias_map: model_alias,
            view,
        }
    }
}

pub(super) struct RequestSetup {
    pub(super) model: String,
    pub(super) facts: RequestFacts,
    pub(super) environment: Option<String>,
    pub(super) specific_deployment: bool,
    pub(super) pinned: bool,
    pub(super) excluded: Vec<String>,
    pub(super) retry_skipped: Vec<String>,
    // the typed prompt and request arguments the prompt caching check reads
    pub(super) messages: Option<Vec<Message>>,
    pub(super) request_kwargs: Option<Object>,
    // the request's routing_strategy entry, which LiteLLM reads before the router's own strategy
    pub(super) routing_strategy: Option<String>,
    pub(super) clients: Option<CachedClients>,
}

impl RequestSetup {
    pub(super) fn new(model: &str) -> Self {
        Self {
            model: model.to_owned(),
            facts: RequestFacts::default(),
            environment: None,
            specific_deployment: false,
            pinned: false,
            excluded: Vec::new(),
            retry_skipped: Vec::new(),
            messages: None,
            request_kwargs: None,
            routing_strategy: None,
            clients: None,
        }
    }

    pub(super) fn request(&self) -> RouteRequest<'_> {
        RouteRequest {
            model: &self.model,
            facts: &self.facts,
            authority: Authority::default(),
            environment: self.environment.as_deref(),
            specific_deployment: self.specific_deployment,
            affinity_pinned: self.pinned,
            excluded_deployment_ids: &self.excluded,
            retry_skipped_deployment_ids: &self.retry_skipped,
            messages: self.messages.as_deref(),
            request_kwargs: self.request_kwargs.as_ref(),
        }
    }

    pub(super) fn client_access(&self) -> Option<ClientAccess<'_>> {
        let clients = self.clients.as_ref()?;
        Some(ClientAccess {
            clients,
            kind: ClientKind::Async,
            stream: false,
        })
    }
}

pub(super) fn seeded() -> StdRng {
    StdRng::seed_from_u64(7)
}

pub(super) fn clock() -> ClientClock {
    Arc::new(SystemTime::now)
}

// The strategy the request names for itself, which _get_routing_context reads before the router's own.
fn override_of(setup: &RequestSetup) -> StrategyOverride<'_> {
    setup
        .routing_strategy
        .as_deref()
        .map_or(StrategyOverride::Absent, StrategyOverride::Text)
}

// The route over a built context, with B395's dispatch resolved once for the request.
async fn drive(
    fixture: &Fixture,
    context: &RouteContext<'_>,
    setup: &RequestSetup,
    strategy: RoutingStrategy,
    random: &mut StdRng,
) -> Result<AttemptResult, RoutingError> {
    let groups = RoutingGroups::default();
    let shadowing = BTreeSet::new();
    let mut dispatch = StrategyDispatch::new(strategy, None).unwrap();
    let mut routing = StrategyRoute {
        dispatch: &mut dispatch,
        override_strategy: override_of(setup),
        scope: RoutingScope {
            groups: &groups,
            shadowing: &shadowing,
        },
    };
    let resolved = {
        let resolver = fixture.resolver();
        prepare(&resolver, context, &mut routing)?
    };
    route(context, routing.dispatch, resolved, random).await
}

pub(super) async fn run_route(
    fixture: &Fixture,
    setup: &RequestSetup,
    services: &dyn RouteServices,
    strategy: RoutingStrategy,
    random: &mut StdRng,
) -> Result<AttemptResult, RoutingError> {
    let request = setup.request();
    let (limits, params) = (NoLimits, Providers::Unknown);
    let mut context = route_context(&request, services, &limits, &params);
    context.clients = setup.client_access();
    drive(fixture, &context, setup, strategy, random).await
}

// A simple-shuffle route over the caller's limits and providers, with the pre-call checks it asks for.
pub(super) async fn run_checked(
    fixture: &Fixture,
    setup: &RequestSetup,
    services: &dyn RouteServices,
    checks: &Checked<'_>,
) -> Result<AttemptResult, RoutingError> {
    let request = setup.request();
    let mut context = route_context(&request, services, checks.limits, checks.params);
    context.policy.enable_pre_call_checks = checks.pre_call;
    context.clients = setup.client_access();
    let mut random = seeded();
    drive(
        fixture,
        &context,
        setup,
        RoutingStrategy::SimpleShuffle,
        &mut random,
    )
    .await
}

pub(super) async fn run_simple(
    fixture: &Fixture,
    setup: &RequestSetup,
) -> Result<AttemptResult, RoutingError> {
    let mut random = seeded();
    run_route(
        fixture,
        setup,
        &NoServices,
        RoutingStrategy::SimpleShuffle,
        &mut random,
    )
    .await
}

pub(super) async fn run_healthy(
    fixture: &Fixture,
    setup: &RequestSetup,
) -> Result<HealthyDeployments, RoutingError> {
    let request = setup.request();
    let (services, limits, params) = (NoServices, NoLimits, Providers::Unknown);
    let context = route_context(&request, &services, &limits, &params);
    let matched = {
        let resolver = fixture.resolver();
        resolve_catalog(&resolver, context.request)?
    };
    healthy_deployments(&context, matched).await
}

// A simple-shuffle route over one window per deployment id, with the pre-call checks as asked.
pub(super) async fn run_windowed(
    fixture: &Fixture,
    setup: &RequestSetup,
    windows: &Windows,
    pre_call: bool,
) -> Result<AttemptResult, RoutingError> {
    let checks = Checked {
        limits: windows,
        params: &Providers::Unknown,
        pre_call,
    };
    run_checked(fixture, setup, &NoServices, &checks).await
}

// The error a simple-shuffle route over the fixture ends with.
pub(super) async fn error_of(fixture: &Fixture, setup: &RequestSetup) -> RoutingError {
    run_simple(fixture, setup).await.unwrap_err()
}

// The no-deployment error a simple-shuffle route over the fixture ends with.
pub(super) async fn no_deployment_of(fixture: &Fixture, setup: &RequestSetup) -> NoDeployment {
    let RoutingError::NoDeployment(no_deployment) = error_of(fixture, setup).await else {
        panic!("an empty pool reports no deployment");
    };
    no_deployment
}

pub(super) fn picked(result: &AttemptResult) -> &str {
    result.deployment.id().unwrap_or_default()
}

// A router whose deployments all cool down, with the cooldown a real cache would report.
pub(super) struct Cooldown {
    pub(super) ids: Vec<String>,
    pub(super) seconds: f64,
}

impl RouteServices for Cooldown {
    fn min_cooldown(&self, _model_ids: &[String]) -> f64 {
        self.seconds
    }

    fn cooldown_list(&self) -> Vec<String> {
        self.ids.clone()
    }
}

// A callback that keeps only the pool's last deployment, so a pick that ignored it lands elsewhere.
pub(super) struct LastOnly;

impl RouteServices for LastOnly {
    fn filter_deployments(
        &self,
        deployments: Vec<DeploymentSpec>,
    ) -> Result<Vec<DeploymentSpec>, Cancelled> {
        Ok(deployments.into_iter().rev().take(1).collect())
    }
}

// A callback the caller cancelled, which stops the route instead of selecting a deployment.
pub(super) struct Cancelling;

impl RouteServices for Cancelling {
    fn filter_deployments(
        &self,
        _deployments: Vec<DeploymentSpec>,
    ) -> Result<Vec<DeploymentSpec>, Cancelled> {
        Err(Cancelled)
    }
}

pub(super) fn body(entries: &[(&str, Json)]) -> Response {
    Response::Object(object(entries))
}

pub(super) fn metadata(id: &str) -> JsonObject {
    object(&[("model_info", Json::Object(object(&[("id", text(id))])))])
}

// The body entry a dict response carries its hidden params under
pub(super) fn hidden(entries: &[(&str, Json)]) -> (&'static str, Json) {
    ("_hidden_params", Json::Object(object(entries)))
}

pub(super) fn hidden_with(model_id: Json) -> JsonObject {
    object(&[(
        "_hidden_params",
        Json::Object(object(&[
            ("model_id", model_id),
            (
                "additional_headers",
                Json::Object(object(&[("x-probe", text("1"))])),
            ),
        ])),
    )])
}
