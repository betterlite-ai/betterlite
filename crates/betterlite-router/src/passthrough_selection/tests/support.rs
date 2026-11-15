use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rand::TryRng;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_catalog::error::CatalogLookupError;
use crate::deployment_catalog::lookup::CatalogResolver;
use crate::deployment_catalog::types::{Authority, CandidateView};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::passthrough_selection::error::PassthroughSelectionError;
use crate::passthrough_selection::select::select;
use crate::passthrough_selection::selector::SelectorCaller;
use crate::passthrough_selection::types::{Selected, SelectionRequest, SelectionServices};
use crate::router_predicates::params::{ParamSupport, ResolvedProvider};
use crate::static_eligibility::filter::EligibilityServices;
use crate::static_eligibility::request::RequestFacts;
use crate::static_eligibility::tokens::TokenLimits;
use crate::strategy_dispatch::groups::RoutingGroups;
use crate::strategy_dispatch::selection::SelectorCall;
use crate::strategy_dispatch::selector::SelectorId;
use crate::strategy_dispatch::service::StrategyDispatch;
use crate::strategy_dispatch::strategy::RoutingStrategy;
use crate::strategy_dispatch::types::{RequestBindings, StrategyOverride};

pub(super) const MODEL: &str = "gpt";
pub(super) const LITELLM_MODEL: &str = "openai/gpt-4o";

pub(super) fn spec(model_name: &str, id: &str, model: &str) -> DeploymentSpec {
    let params = format!(r#"{{"model":"{model}"}}"#);
    let info = JsonObject::from_iter([("id".to_owned(), Json::Text(id.to_owned()))]);
    DeploymentSpec::new(
        model_name.to_owned(),
        serde_json::from_str(&params).unwrap_or_default(),
        info,
        JsonObject::new(),
        String::new,
    )
    .unwrap_or_else(|error| panic!("the test deployment is invalid: {error}"))
}

pub(super) fn gpt(id: &str) -> DeploymentSpec {
    spec(MODEL, id, LITELLM_MODEL)
}

pub(super) fn with_param(mut deployment: DeploymentSpec, key: &str, value: Json) -> DeploymentSpec {
    deployment.litellm_params.insert(key.to_owned(), value);
    deployment
}

pub(super) fn flagged(deployment: DeploymentSpec) -> DeploymentSpec {
    with_param(deployment, "use_in_pass_through", flag(true))
}

pub(super) fn supported_in(
    mut deployment: DeploymentSpec,
    environments: &[&str],
) -> DeploymentSpec {
    deployment.model_info.insert(
        "supported_environments".to_owned(),
        Json::List(
            environments
                .iter()
                .map(|name| Json::Text((*name).to_owned()))
                .collect(),
        ),
    );
    deployment
}

pub(super) fn weighted(id: &str, weight: u64) -> DeploymentSpec {
    with_param(flagged(gpt(id)), "weight", number(weight))
}

pub(super) const fn flag(value: bool) -> Json {
    Json::Bool(value)
}

pub(super) fn number(value: u64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

pub(super) fn signed(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn message(content: &str) -> Json {
    serde_json::from_str(&format!(r#"{{"role":"user","content":"{content}"}}"#))
        .unwrap_or(Json::Null)
}

pub(super) fn facts() -> RequestFacts {
    RequestFacts::default()
}

pub(super) fn json_mode() -> RequestFacts {
    RequestFacts {
        messages: Some(vec![message("hi")]),
        response_format: true,
        ..RequestFacts::default()
    }
}

fn catalog(deployments: Vec<DeploymentSpec>) -> DeploymentCatalog {
    let mut catalog = DeploymentCatalog::default();
    for deployment in deployments {
        catalog.model_names.insert(deployment.model_name.clone());
        catalog.index_patterns(&deployment).unwrap();
        catalog.push(deployment);
    }
    catalog
}

// The probe router has no access groups, reserved models or default fallbacks.
struct NoPolicy;

impl CandidateView for NoPolicy {
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

struct NoLimits;

impl TokenLimits for NoLimits {
    fn max_input_tokens(&self, _: &DeploymentSpec, _: &str) -> Option<usize> {
        None
    }
}

// The probe's parameter support: gpt-4 and gpt-3.5-turbo-16k are the two models the OpenAI config leaves response_format out of.
struct Params;

impl ParamSupport for Params {
    fn is_authenticating_provider(&self, _: &str, _: Option<&str>) -> bool {
        false
    }

    fn resolve_provider(&self, model: &str, _: Option<&str>) -> Option<ResolvedProvider> {
        match model.split_once('/') {
            Some((provider, bare)) => Some(ResolvedProvider {
                model: bare.to_owned(),
                custom_llm_provider: provider.to_owned(),
            }),
            None => Some(ResolvedProvider {
                model: model.to_owned(),
                custom_llm_provider: "openai".to_owned(),
            }),
        }
    }

    fn supported_openai_params(
        &self,
        model: &str,
        custom_llm_provider: &str,
        _: Option<&str>,
    ) -> Option<Vec<String>> {
        if custom_llm_provider != "openai" {
            return None;
        }
        match model {
            "gpt-4" | "gpt-3.5-turbo-16k" => Some(Vec::new()),
            _ => Some(vec!["response_format".to_owned()]),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Recorded {
    pub(super) selector: SelectorId,
    pub(super) call: SelectorCall,
    pub(super) available: usize,
}

#[derive(Default)]
pub(super) struct Picker {
    pub(super) result: Option<usize>,
    pub(super) calls: Vec<Recorded>,
}

impl SelectorCaller for Picker {
    fn pick(
        &mut self,
        call: SelectorCall,
        selector: SelectorId,
        deployments: &[DeploymentSpec],
        _: &RequestFacts,
    ) -> Option<usize> {
        self.calls.push(Recorded {
            selector,
            call,
            available: deployments.len(),
        });
        self.result
    }
}

// Python's sample 0: the 53-bit draw becomes 1 as u32 and 2047 as u64, both inside every range.
struct SampleZero;

impl TryRng for SampleZero {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(1)
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(2047)
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        dst.fill(0);
        Ok(())
    }
}

pub(super) struct Lookup<'request> {
    model: &'request str,
    facts: &'request RequestFacts,
    specific_deployment: bool,
    strategy: StrategyOverride<'request>,
}

impl<'request> Lookup<'request> {
    pub(super) fn with_specific(mut self) -> Self {
        self.specific_deployment = true;
        self
    }

    pub(super) fn with_strategy(mut self, strategy: StrategyOverride<'request>) -> Self {
        self.strategy = strategy;
        self
    }
}

pub(super) fn ask<'request>(
    model: &'request str,
    facts: &'request RequestFacts,
) -> Lookup<'request> {
    Lookup {
        model,
        facts,
        specific_deployment: false,
        strategy: StrategyOverride::Absent,
    }
}

pub(super) struct World {
    pub(super) dispatch: StrategyDispatch,
    pub(super) catalog: DeploymentCatalog,
    pub(super) model_group_alias: BTreeMap<String, String>,
    pub(super) model_alias_map: BTreeMap<String, String>,
    pub(super) groups: RoutingGroups,
    pub(super) environment: &'static str,
    pub(super) enable_pre_call_checks: bool,
    pub(super) drop_params: bool,
}

pub(super) fn world_with(strategy: RoutingStrategy, deployments: Vec<DeploymentSpec>) -> World {
    World {
        dispatch: StrategyDispatch::new(strategy, None).unwrap(),
        catalog: catalog(deployments),
        model_group_alias: BTreeMap::new(),
        model_alias_map: BTreeMap::new(),
        groups: RoutingGroups::default(),
        environment: "production",
        enable_pre_call_checks: false,
        drop_params: false,
    }
}

pub(super) fn shuffle(deployments: Vec<DeploymentSpec>) -> World {
    world_with(RoutingStrategy::SimpleShuffle, deployments)
}

// The MODEL group reachable as the group alias "alias".
pub(super) fn aliased(deployments: Vec<DeploymentSpec>, strategy: RoutingStrategy) -> World {
    let mut world = world_with(strategy, deployments);
    world
        .model_group_alias
        .insert("alias".to_owned(), MODEL.to_owned());
    world
}

impl World {
    pub(super) fn call(
        &mut self,
        picker: &mut dyn SelectorCaller,
        lookup: &Lookup<'_>,
    ) -> Result<Selected, PassthroughSelectionError> {
        let resolver = CatalogResolver {
            catalog: &self.catalog,
            model_group_alias: &self.model_group_alias,
            model_alias_map: &self.model_alias_map,
            view: &NoPolicy,
        };
        let limits = NoLimits;
        let params = Params;
        let eligibility = EligibilityServices {
            limits: &limits,
            params: &params,
        };
        let shadowing = BTreeSet::new();
        let services = SelectionServices {
            catalog: &resolver,
            eligibility: &eligibility,
            groups: &self.groups,
            shadowing: &shadowing,
            environment: Some(self.environment),
            enable_pre_call_checks: self.enable_pre_call_checks,
            drop_params: self.drop_params,
        };
        let mut bindings = RequestBindings::default();
        let mut request = SelectionRequest {
            model: lookup.model,
            specific_deployment: lookup.specific_deployment,
            authority: Authority {
                team_id: None,
                proxy_admin: false,
            },
            facts: lookup.facts,
            strategy: lookup.strategy,
            bindings: Some(&mut bindings),
        };
        let mut random = SampleZero;
        select(
            &mut self.dispatch,
            picker,
            &mut request,
            &services,
            &mut random,
        )
    }
}

pub(super) fn assert_selected(selected: &Selected, model_name: &str, id: &str) {
    assert_eq!(selected.deployment.model_name, model_name);
    assert_eq!(selected.deployment.id(), Some(id));
    assert_eq!(
        selected.deployment.litellm_params.get("model"),
        Some(&Json::Text(LITELLM_MODEL.to_owned()))
    );
}

pub(super) fn assert_not_configured(
    result: Result<Selected, PassthroughSelectionError>,
    model: &str,
) {
    let Err(error) = result else {
        panic!("the selection should fail");
    };
    let PassthroughSelectionError::NotConfigured { model: named } = &error else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(named, model);
    assert_eq!(
        error.to_string(),
        format!(
            "Model {model} has no deployments configured with use_in_pass_through=True. Please add use_in_pass_through: true to the deployment configuration"
        )
    );
}

pub(super) fn assert_rate_limited(
    result: Result<Selected, PassthroughSelectionError>,
    model: &str,
) {
    let Err(error) = result else {
        panic!("the selection should fail");
    };
    let PassthroughSelectionError::NoDeployment { model: named } = &error else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(named, model);
}

pub(super) fn assert_invalid_field(
    result: Result<Selected, PassthroughSelectionError>,
    deployment_id: &str,
    field: &'static str,
    expected: &'static str,
) {
    let Err(error) = result else {
        panic!("the selection should fail");
    };
    let PassthroughSelectionError::InvalidDeploymentField {
        deployment_id: named_id,
        field: named_field,
        expected: named_expected,
    } = &error
    else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(named_id, deployment_id);
    assert_eq!(*named_field, field);
    assert_eq!(*named_expected, expected);
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Metric {
    Weight,
    Rpm,
    Tpm,
}

impl Metric {
    fn key(self) -> &'static str {
        match self {
            Self::Weight => "weight",
            Self::Rpm => "rpm",
            Self::Tpm => "tpm",
        }
    }
}

pub(super) fn metric_setting(metric: Metric, id: &str, value: u64) -> DeploymentSpec {
    with_param(flagged(gpt(id)), metric.key(), number(value))
}
