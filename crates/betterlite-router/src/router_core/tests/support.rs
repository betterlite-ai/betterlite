use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
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
use crate::router_core::pipeline::{HealthyDeployments, StrategyRoute, healthy_deployments, route};
use crate::router_core::route::{ClientAccess, RouteRequest};
use crate::router_core::services::{Cancelled, RouteServices};
use crate::router_core::test_support::{NoLimits, NoParams, NoServices, route_context};
use crate::static_eligibility::request::RequestFacts;
use crate::strategy_dispatch::groups::{RoutingGroups, RoutingScope};
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::StrategyOverride;

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
        CatalogResolver {
            catalog: &self.catalog,
            model_group_alias: &self.group_alias,
            model_alias_map: &self.model_alias,
            view: &self.view,
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

pub(super) async fn run_route(
    fixture: &Fixture,
    setup: &RequestSetup,
    services: &dyn RouteServices,
    strategy: RoutingStrategy,
    random: &mut StdRng,
) -> Result<AttemptResult, RoutingError> {
    let request = setup.request();
    let (limits, params) = (NoLimits, NoParams);
    let mut context = route_context(&request, services, &limits, &params);
    context.clients = setup.client_access();
    let groups = RoutingGroups::default();
    let shadowing = BTreeSet::new();
    let mut dispatch = StrategyDispatch::new(strategy, None).unwrap();
    let routing = StrategyRoute {
        dispatch: &mut dispatch,
        override_strategy: StrategyOverride::Absent,
        scope: RoutingScope {
            groups: &groups,
            shadowing: &shadowing,
        },
    };
    let resolver = fixture.resolver();
    route(&resolver, &context, routing, random).await
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
    let (services, limits, params) = (NoServices, NoLimits, NoParams);
    let context = route_context(&request, &services, &limits, &params);
    let resolver = fixture.resolver();
    healthy_deployments(&resolver, &context).await
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
