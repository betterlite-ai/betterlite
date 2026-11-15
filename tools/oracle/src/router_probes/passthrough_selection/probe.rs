use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_catalog::lookup::CatalogResolver;
use betterlite_router::deployment_catalog::types::Authority;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::passthrough_selection::error::PassthroughSelectionError;
use betterlite_router::passthrough_selection::select::select;
use betterlite_router::passthrough_selection::types::{
    Selected, SelectionRequest, SelectionServices,
};
use betterlite_router::static_eligibility::error::EligibilityError;
use betterlite_router::static_eligibility::filter::EligibilityServices;
use betterlite_router::static_eligibility::request::RequestFacts;
use betterlite_router::strategy_dispatch::groups::RoutingGroups;
use betterlite_router::strategy_dispatch::service::StrategyDispatch;
use betterlite_router::strategy_dispatch::types::{RequestBindings, StrategyOverride};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

use super::{Entry, Found, Input, Lookup, Outcome, Output, Picked};

mod support;

use support::{NoLimits, NoPolicy, NoSelector, ProbeSupport};

fn facts(lookup: &Lookup) -> RequestFacts {
    RequestFacts {
        messages: lookup.messages.clone(),
        input: None,
        instructions: None,
        system: None,
        tools: Vec::new(),
        response_format: lookup.response_format,
        target_order: None,
        input_token_count: None,
        compaction_pending: false,
    }
}

fn invalid<E>(error: E) -> OperationError
where
    E: Display,
{
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}

fn info(entry: &Entry) -> JsonObject {
    let mut info = JsonObject::new();
    info.insert("id".to_owned(), Json::Text(entry.id.clone()));
    if let Some(blocked) = entry.blocked {
        info.insert("blocked".to_owned(), Json::Bool(blocked));
    }
    if let Some(environments) = &entry.supported_environments {
        let names = environments.iter().map(|name| Json::Text(name.clone()));
        info.insert(
            "supported_environments".to_owned(),
            Json::List(names.collect()),
        );
    }
    info
}

fn params(entry: &Entry) -> JsonObject {
    let mut fields = vec![
        ("model".to_owned(), Json::Text(entry.model.clone())),
        ("api_key".to_owned(), Json::Text("sk-probe".to_owned())),
    ];
    if let Some(pass_through) = entry.use_in_pass_through {
        fields.push(("use_in_pass_through".to_owned(), Json::Bool(pass_through)));
    }
    if let Some(weight) = entry.weight {
        fields.push((
            "weight".to_owned(),
            Json::Number(serde_json::Number::from(weight)),
        ));
    }
    if let Some(rpm) = entry.rpm {
        fields.push((
            "rpm".to_owned(),
            Json::Number(serde_json::Number::from(rpm)),
        ));
    }
    if let Some(tpm) = entry.tpm {
        fields.push((
            "tpm".to_owned(),
            Json::Number(serde_json::Number::from(tpm)),
        ));
    }
    if let Some(order) = entry.order {
        fields.push((
            "order".to_owned(),
            Json::Number(serde_json::Number::from(order)),
        ));
    }
    JsonObject::from_iter(fields)
}

fn spec(entry: &Entry) -> Result<DeploymentSpec, OperationError> {
    DeploymentSpec::new(
        entry.model_name.clone(),
        params(entry),
        info(entry),
        JsonObject::new(),
        String::new,
    )
    .map_err(invalid)
}

fn catalog(entries: &[Entry]) -> Result<DeploymentCatalog, OperationError> {
    let mut catalog = DeploymentCatalog::default();
    for entry in entries {
        let built = spec(entry)?;
        catalog.model_names.insert(built.model_name.clone());
        catalog.index_patterns(&built).map_err(invalid)?;
        catalog.push(built);
    }
    Ok(catalog)
}

fn picked(deployment: &DeploymentSpec) -> Picked {
    let model = match deployment.litellm_params.get("model") {
        Some(Json::Text(model)) => model.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => String::new(),
    };
    Picked {
        model_name: deployment.model_name.clone(),
        model,
        id: deployment.id().unwrap_or_default().to_owned(),
    }
}

fn selection_request<'request>(
    lookup: &'request Lookup,
    facts: &'request RequestFacts,
    bindings: &'request mut RequestBindings,
) -> SelectionRequest<'request> {
    SelectionRequest {
        model: &lookup.model,
        specific_deployment: lookup.specific_deployment,
        authority: Authority {
            team_id: None,
            proxy_admin: false,
        },
        facts,
        strategy: lookup
            .routing_strategy
            .map_or(StrategyOverride::Absent, |strategy| {
                StrategyOverride::Text(strategy.routing().wire_name())
            }),
        bindings: Some(bindings),
    }
}

// LiteLLM raises its no-deployments error, a 503 for a paused deployment and a 400 for everything else.
fn found(
    result: Result<Selected, PassthroughSelectionError>,
    random: &InjectedRandom<'_>,
) -> Result<Found, OperationError> {
    if random.exhausted {
        return Err(OperationError::SamplesExhausted {
            message: "the request has no sample left for this draw".to_owned(),
        });
    }
    let (outcome, selected) = match result {
        Ok(selected) => (Outcome::Selected, Some(picked(&selected.deployment))),
        Err(PassthroughSelectionError::NoDeployment { .. }) => (Outcome::RateLimited, None),
        Err(PassthroughSelectionError::Eligibility(EligibilityError::Blocked { .. })) => {
            (Outcome::ServiceUnavailable, None)
        }
        Err(_) => (Outcome::BadRequest, None),
    };
    Ok(Found { outcome, selected })
}

struct Probe {
    dispatch: StrategyDispatch,
    catalog: DeploymentCatalog,
    groups: RoutingGroups,
    shadowing: BTreeSet<String>,
    enable_pre_call_checks: bool,
    drop_params: bool,
}

impl Probe {
    fn answer(
        &mut self,
        lookup: &Lookup,
        random: &mut InjectedRandom<'_>,
    ) -> Result<Found, OperationError> {
        let request_facts = facts(lookup);
        let result = self.pick_deployment(lookup, &request_facts, random);
        found(result, random)
    }

    fn pick_deployment(
        &mut self,
        lookup: &Lookup,
        facts: &RequestFacts,
        random: &mut InjectedRandom<'_>,
    ) -> Result<Selected, PassthroughSelectionError> {
        let empty = BTreeMap::new();
        let resolver = CatalogResolver {
            catalog: &self.catalog,
            model_group_alias: &empty,
            model_alias_map: &empty,
            view: &NoPolicy,
        };
        let limits = NoLimits;
        let params = ProbeSupport;
        let eligibility = EligibilityServices {
            limits: &limits,
            params: &params,
        };
        let services = SelectionServices {
            catalog: &resolver,
            eligibility: &eligibility,
            groups: &self.groups,
            shadowing: &self.shadowing,
            environment: Some("production"),
            enable_pre_call_checks: self.enable_pre_call_checks,
            drop_params: self.drop_params,
        };
        let mut bindings = RequestBindings::default();
        let mut request = selection_request(lookup, facts, &mut bindings);
        let mut picker = NoSelector;
        select(
            &mut self.dispatch,
            &mut picker,
            &mut request,
            &services,
            random,
        )
    }
}

pub(crate) fn run(
    input: &Input,
    random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let mut probe = Probe {
        dispatch: StrategyDispatch::new(input.strategy.routing(), None).map_err(invalid)?,
        catalog: catalog(&input.models)?,
        groups: RoutingGroups::default(),
        shadowing: BTreeSet::new(),
        enable_pre_call_checks: input.enable_pre_call_checks,
        drop_params: input.drop_params,
    };
    let mut lookups = Vec::new();
    for lookup in &input.lookups {
        lookups.push(probe.answer(lookup, random)?);
    }
    Ok(ProbeOutput::PassthroughSelection(Output { lookups }))
}
