use std::collections::BTreeMap;
use std::fmt::Display;

use async_trait::async_trait;
use betterlite_config::loader::ConfigLoader;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_catalog::types::Candidates;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::lar1_routing::health::HealthyDeployments;
use betterlite_router::lar1_routing::strategy::Lar1Strategy;
use betterlite_router::router_core::error::RoutingError;
use betterlite_server::routing::lar1_routing::resolve;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

const API_KEY: &str = "sk-probe";
// a probe message stops at 1 MiB, so a case builds its oversized metadata from the padding on each side
const MAX_PADDING_BYTES: u64 = 1 << 22;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub model: String,
    #[serde(default = "default_group")]
    pub group: String,
    #[serde(default)]
    pub specific_deployment: bool,
    pub deployments: Vec<Entry>,
    #[serde(default)]
    pub routing_strategy_args: Option<String>,
    #[serde(default)]
    pub metadata: Option<String>,
    #[serde(default)]
    pub padding_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub model: String,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub selected: Option<String>,
    pub error: Option<String>,
}

struct ProbeRouter {
    group: String,
    deployments: Vec<DeploymentSpec>,
}

fn default_group() -> String {
    "probe-group".to_owned()
}

fn invalid<E>(error: E) -> OperationError
where
    E: Display,
{
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}

fn litellm_model(deployment: &DeploymentSpec) -> Option<&str> {
    match deployment.litellm_params.get("model") {
        Some(Json::Text(model)) => Some(model),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// Router.async_get_healthy_deployments: a model id answers one deployment and every other request
// answers the pool in model_list order, a named deployment included
#[async_trait]
impl HealthyDeployments for ProbeRouter {
    async fn healthy_deployments(
        &self,
        model: &str,
        specific_deployment: bool,
    ) -> Result<Candidates, RoutingError> {
        if !specific_deployment && model == self.group {
            return Ok(Candidates::Pool(self.deployments.clone()));
        }
        if !specific_deployment {
            let by_id = self
                .deployments
                .iter()
                .find(|deployment| deployment.id() == Some(model));
            if let Some(deployment) = by_id {
                return Ok(Candidates::Single(Box::new(deployment.clone())));
            }
        }
        let pool = self
            .deployments
            .iter()
            .filter(|deployment| litellm_model(deployment) == Some(model))
            .cloned()
            .collect();
        Ok(Candidates::Pool(pool))
    }
}

fn deployment(group: &str, entry: &Entry) -> Result<DeploymentSpec, OperationError> {
    let mut params = JsonObject::new();
    params.insert("model".to_owned(), Json::Text(entry.model.clone()));
    params.insert("api_key".to_owned(), Json::Text(API_KEY.to_owned()));
    let mut model_info = JsonObject::new();
    model_info.insert("id".to_owned(), Json::Text(entry.id.clone()));
    if let Some(kind) = &entry.kind {
        model_info.insert("type".to_owned(), Json::Text(kind.clone()));
    }
    DeploymentSpec::new(
        group.to_owned(),
        params,
        model_info,
        JsonObject::new(),
        String::new,
    )
    .map_err(invalid)
}

// the configured arguments go through the production config section and the startup resolver
fn strategy(args: Option<&str>) -> Result<Lar1Strategy, String> {
    let source = args.map_or_else(
        || "router_settings: {}\n".to_owned(),
        |text| format!("router_settings:\n  routing_strategy_args: {text}\n"),
    );
    let lookup = BTreeMap::<String, String>::new();
    let config = ConfigLoader { lookup }
        .load(&source)
        .map_err(|error| error.to_string())?;
    resolve(&config.sections.lar1_routing).map_err(|error| error.to_string())
}

fn metadata(input: &Input) -> Result<Option<Vec<u8>>, OperationError> {
    let Some(text) = &input.metadata else {
        return Ok(None);
    };
    if input.padding_bytes > MAX_PADDING_BYTES {
        return Err(invalid(format!(
            "the padding is {} bytes, over the {MAX_PADDING_BYTES} byte limit",
            input.padding_bytes
        )));
    }
    let padding = usize::try_from(input.padding_bytes).map_err(invalid)?;
    let mut bytes = text.as_bytes().to_vec();
    bytes.resize(bytes.len().saturating_add(padding), b' ');
    Ok(Some(bytes))
}

async fn execute(input: &Input) -> Result<Output, OperationError> {
    let lar1 = match strategy(input.routing_strategy_args.as_deref()) {
        Ok(built) => built,
        Err(error) => {
            return Ok(Output {
                selected: None,
                error: Some(error),
            });
        }
    };
    let deployments = input
        .deployments
        .iter()
        .map(|entry| deployment(&input.group, entry))
        .collect::<Result<Vec<_>, _>>()?;
    let router = ProbeRouter {
        group: input.group.clone(),
        deployments,
    };
    let fragment = metadata(input)?;
    let outcome = lar1
        .select_deployment(
            Some(&router),
            fragment.as_deref(),
            input.specific_deployment,
            &input.model,
        )
        .await;
    Ok(match outcome {
        Ok(selected) => Output {
            selected: selected.and_then(|deployment| deployment.id().map(str::to_owned)),
            error: None,
        },
        Err(error) => Output {
            selected: None,
            error: Some(error.to_string()),
        },
    })
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(invalid)?;
    let output = runtime.block_on(execute(input))?;
    Ok(ProbeOutput::Lar1Routing(output))
}
