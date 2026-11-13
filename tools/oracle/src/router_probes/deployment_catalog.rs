use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_catalog::error::CatalogLookupError;
use betterlite_router::deployment_catalog::lookup::CatalogResolver;
use betterlite_router::deployment_catalog::scope::litellm_model;
use betterlite_router::deployment_catalog::types::{Authority, CandidateView, LookupRequest};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub model_name: String,
    pub model: String,
    pub id: String,
    pub team_id: Option<String>,
    pub team_public_model_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lookup {
    pub model: String,
    pub specific_deployment: bool,
    pub team_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub models: Vec<Entry>,
    pub model_group_alias: BTreeMap<String, String>,
    pub lookups: Vec<Lookup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selected {
    pub model_name: String,
    pub model: String,
    pub id: String,
}

// LiteLLM answers an empty pool with a RouterRateLimitError further down the selection, so the probe names it apart from a bad request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Selected,
    BadRequest,
    EmptyPool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Found {
    pub outcome: Outcome,
    pub selected: Option<Selected>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub lookups: Vec<Found>,
}

// The probe router has no access groups, reserved models, routing groups, default fallbacks or provider inference.
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

fn spec(entry: &Entry) -> Result<DeploymentSpec, OperationError> {
    let mut info = JsonObject::from_iter([("id".to_owned(), Json::Text(entry.id.clone()))]);
    for (key, value) in [
        ("team_id", &entry.team_id),
        ("team_public_model_name", &entry.team_public_model_name),
    ] {
        if let Some(text) = value {
            info.insert(key.to_owned(), Json::Text(text.clone()));
        }
    }
    let params = JsonObject::from_iter([
        ("model".to_owned(), Json::Text(entry.model.clone())),
        ("api_key".to_owned(), Json::Text("sk-probe".to_owned())),
    ]);
    DeploymentSpec::new(
        entry.model_name.clone(),
        params,
        info,
        JsonObject::new(),
        String::new,
    )
    .map_err(|error| OperationError::InvalidRequest {
        message: error.to_string(),
    })
}

fn answer(resolver: &CatalogResolver<'_>, lookup: &Lookup) -> Result<Found, OperationError> {
    let request = LookupRequest {
        model: &lookup.model,
        specific_deployment: lookup.specific_deployment,
        authority: Authority {
            team_id: lookup.team_id.as_deref(),
            proxy_admin: false,
        },
    };
    let Ok(matched) = resolver.resolve(&request) else {
        return Ok(Found {
            outcome: Outcome::BadRequest,
            selected: None,
        });
    };
    match matched.candidates.deployments() {
        [] => Ok(Found {
            outcome: Outcome::EmptyPool,
            selected: None,
        }),
        [only] => Ok(Found {
            outcome: Outcome::Selected,
            selected: Some(Selected {
                model_name: only.model_name.clone(),
                model: litellm_model(only).to_owned(),
                id: only.id().unwrap_or_default().to_owned(),
            }),
        }),
        _ => Err(OperationError::InvalidRequest {
            message: "a probe lookup must match at most one deployment".to_owned(),
        }),
    }
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let mut catalog = DeploymentCatalog::default();
    for entry in &input.models {
        let built = spec(entry)?;
        catalog.model_names.insert(built.model_name.clone());
        catalog
            .index_patterns(&built)
            .map_err(|error| OperationError::InvalidRequest {
                message: error.to_string(),
            })?;
        catalog.push(built);
    }
    let resolver = CatalogResolver {
        catalog: &catalog,
        model_group_alias: &input.model_group_alias,
        model_alias_map: &BTreeMap::new(),
        view: &NoPolicy,
    };
    let lookups = input
        .lookups
        .iter()
        .map(|lookup| answer(&resolver, lookup))
        .collect::<Result<Vec<Found>, OperationError>>()?;
    Ok(ProbeOutput::DeploymentCatalog(Output { lookups }))
}
