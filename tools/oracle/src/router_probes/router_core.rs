use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_catalog::error::CatalogLookupError;
use betterlite_router::deployment_catalog::lookup::CatalogResolver;
use betterlite_router::deployment_catalog::types::{Authority, CandidateView};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::router_core::attempt::AttemptResult;
use betterlite_router::router_core::error::RoutingError;
use betterlite_router::router_core::headers::{
    Response, apply_response_model_id, ensure_additional_headers, prepare_response,
    response_has_hidden_params,
};
use betterlite_router::router_core::no_deployment::NoDeployment;
use betterlite_router::router_core::pipeline::{self, StrategyRoute};
use betterlite_router::router_core::route::{RouteContext, RoutePolicy, RouteRequest};
use betterlite_router::router_core::test_support::{NoLimits, NoParams, NoServices};
use betterlite_router::static_eligibility::error::EligibilityError;
use betterlite_router::static_eligibility::request::RequestFacts;
use betterlite_router::strategy_dispatch::groups::{RoutingGroups, RoutingScope};
use betterlite_router::strategy_dispatch::service::StrategyDispatch;
use betterlite_router::strategy_dispatch::strategy::RoutingStrategy;
use betterlite_router::strategy_dispatch::types::StrategyOverride;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use tokio::runtime::Builder;
const API_KEY: &str = "sk-probe";
const ENVIRONMENT: &str = "production";
const EXHAUSTED: &str = "the request has no sample left for this draw";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub model_name: String,
    pub model: String,
    pub id: String,
    pub weight: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub model: String,
    #[serde(default)]
    pub excluded_deployment_ids: Vec<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Carrier {
    Dict,
    Attributed,
    Stream,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub carrier: Carrier,
    pub hidden: Option<JsonObject>,
    pub metadata: Option<JsonObject>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub models: Vec<Entry>,
    #[serde(default)]
    pub routes: Vec<Route>,
    #[serde(default)]
    pub responses: Vec<Scenario>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selected {
    pub model_name: String,
    pub model: String,
    pub id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observed {
    pub error_type: String,
    pub model: String,
    pub cooldown_time: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Selected,
    BadRequest,
    RateLimited,
    ServiceUnavailable,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    pub outcome: Outcome,
    pub selected: Option<Selected>,
    pub error: Option<Observed>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseFound {
    pub prepared: bool,
    pub has_hidden: bool,
    pub hidden: JsonObject,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    pub routes: Vec<Found>,
    pub responses: Vec<ResponseFound>,
}

fn invalid<E>(error: E) -> OperationError
where
    E: Display,
{
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}
fn catalog(entries: &[Entry]) -> Result<DeploymentCatalog, OperationError> {
    let mut catalog = DeploymentCatalog::default();
    for entry in entries {
        let mut params = JsonObject::new();
        params.insert("model".to_owned(), Json::Text(entry.model.clone()));
        params.insert("api_key".to_owned(), Json::Text(API_KEY.to_owned()));
        if let Some(weight) = entry.weight {
            params.insert(
                "weight".to_owned(),
                Json::Number(serde_json::Number::from(weight)),
            );
        }
        let info = JsonObject::from_iter([("id".to_owned(), Json::Text(entry.id.clone()))]);
        let built = DeploymentSpec::new(
            entry.model_name.clone(),
            params,
            info,
            JsonObject::new(),
            String::new,
        )
        .map_err(invalid)?;
        catalog.model_names.insert(built.model_name.clone());
        catalog.index_patterns(&built).map_err(invalid)?;
        catalog.push(built);
    }
    Ok(catalog)
}
fn picked(deployment: &DeploymentSpec) -> Selected {
    let model = match deployment.litellm_params.get("model") {
        Some(Json::Text(model)) => model.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => String::new(),
    };
    Selected {
        model_name: deployment.model_name.clone(),
        model,
        id: deployment.id().unwrap_or_default().to_owned(),
    }
}
fn observed(error: &NoDeployment) -> Observed {
    Observed {
        error_type: error.error_type.wire_name().to_owned(),
        model: error.model.clone(),
        cooldown_time: serde_json::Number::from_f64(error.cooldown_time)
            .map_or_else(String::new, |number| number.to_string()),
    }
}
fn selection(outcome: Outcome, selected: Option<Selected>) -> Found {
    Found {
        outcome,
        selected,
        error: None,
    }
}

// LiteLLM raises its no-deployments error, a 503 for a paused deployment and a 400 for everything else.
fn found(
    result: Result<AttemptResult, RoutingError>,
    random: &InjectedRandom<'_>,
) -> Result<Found, OperationError> {
    if random.exhausted {
        return Err(OperationError::SamplesExhausted {
            message: EXHAUSTED.to_owned(),
        });
    }
    Ok(match result {
        Ok(attempt) => selection(Outcome::Selected, Some(picked(&attempt.deployment))),
        Err(RoutingError::NoDeployment(error)) => Found {
            outcome: Outcome::RateLimited,
            selected: None,
            error: Some(observed(&error)),
        },
        Err(RoutingError::Eligibility(EligibilityError::Blocked { .. })) => {
            selection(Outcome::ServiceUnavailable, None)
        }
        Err(_) => selection(Outcome::BadRequest, None),
    })
}

// prepare_response over each carrier a caller can hand the router
fn response(scenario: &Scenario) -> Response {
    match scenario.carrier {
        Carrier::Dict => {
            let mut body = JsonObject::new();
            if let Some(hidden) = &scenario.hidden {
                body.insert("_hidden_params".to_owned(), Json::Object(hidden.clone()));
            }
            Response::Object(body)
        }
        Carrier::Attributed => Response::Attributed(scenario.hidden.clone().unwrap_or_default()),
        Carrier::Stream => Response::Stream,
    }
}
fn observe(scenario: &Scenario) -> ResponseFound {
    let Some(mut prepared) = prepare_response(response(scenario)) else {
        return ResponseFound {
            prepared: false,
            has_hidden: false,
            hidden: JsonObject::new(),
        };
    };
    ensure_additional_headers(&mut prepared);
    apply_response_model_id(&mut prepared, scenario.metadata.as_ref());
    let has_hidden = response_has_hidden_params(&prepared);
    let hidden = if has_hidden {
        prepared.hidden_params()
    } else {
        JsonObject::new()
    };
    ResponseFound {
        prepared: true,
        has_hidden,
        hidden,
    }
}

// The probe router has no access groups, reserved models, default fallbacks or provider inference.
struct OpenView;
impl CandidateView for OpenView {
    fn routing_group(&self, _: &str, _: Option<&str>) -> Option<Vec<DeploymentSpec>> {
        None
    }
    fn filter_access_groups(
        &self,
        _: &str,
        deployments: Vec<DeploymentSpec>,
        _: Option<&str>,
    ) -> Vec<DeploymentSpec> {
        deployments
    }
    fn filter_reserved(
        &self,
        deployments: Vec<DeploymentSpec>,
        _: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, CatalogLookupError> {
        Ok(deployments)
    }
    fn default_fallback(&self) -> Option<String> {
        None
    }
    fn inferred_provider(&self, _: &str) -> Option<String> {
        None
    }
}

fn route_request<'request>(
    route: &'request Route,
    facts: &'request RequestFacts,
) -> RouteRequest<'request> {
    RouteRequest {
        model: &route.model,
        facts,
        authority: Authority::default(),
        environment: Some(ENVIRONMENT),
        specific_deployment: false,
        affinity_pinned: false,
        excluded_deployment_ids: &route.excluded_deployment_ids,
        retry_skipped_deployment_ids: &[],
    }
}

struct Probe {
    dispatch: StrategyDispatch,
    catalog: DeploymentCatalog,
    groups: RoutingGroups,
    shadowing: BTreeSet<String>,
}
impl Probe {
    async fn answer(
        &mut self,
        route: &Route,
        random: &mut InjectedRandom<'_>,
    ) -> Result<Found, OperationError> {
        let facts = RequestFacts::default();
        let request = route_request(route, &facts);
        let aliases = BTreeMap::new();
        let (services, limits, params) = (NoServices, NoLimits, NoParams);
        let resolver = CatalogResolver {
            catalog: &self.catalog,
            model_group_alias: &aliases,
            model_alias_map: &aliases,
            view: &OpenView,
        };
        let context = RouteContext {
            request: &request,
            services: &services,
            limits: &limits,
            params: &params,
            policy: RoutePolicy::default(),
            clients: None,
        };
        let routing = StrategyRoute {
            dispatch: &mut self.dispatch,
            override_strategy: StrategyOverride::Absent,
            scope: RoutingScope {
                groups: &self.groups,
                shadowing: &self.shadowing,
            },
        };
        let result = pipeline::route(&resolver, &context, routing, random).await;
        found(result, random)
    }
}

async fn execute(input: &Input, random: &mut InjectedRandom<'_>) -> Result<Output, OperationError> {
    let mut probe = Probe {
        dispatch: StrategyDispatch::new(RoutingStrategy::SimpleShuffle, None).map_err(invalid)?,
        catalog: catalog(&input.models)?,
        groups: RoutingGroups::default(),
        shadowing: BTreeSet::new(),
    };
    let mut routes = Vec::new();
    for route in &input.routes {
        routes.push(probe.answer(route, random).await?);
    }
    Ok(Output {
        routes,
        responses: input.responses.iter().map(observe).collect(),
    })
}

pub(crate) fn run(
    input: &Input,
    random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(invalid)?;
    let output = runtime.block_on(execute(input, random))?;
    Ok(ProbeOutput::RouterCore(output))
}
