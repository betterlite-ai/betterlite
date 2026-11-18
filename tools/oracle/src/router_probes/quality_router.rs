use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::quality_router as section;
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;
use betterlite_router::quality_router::{QualityRouter, Route};
use betterlite_router::routing_decision_headers::{RoutingValue, apply_quality_headers};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    // the model_list document a config file holds, read through the section a deployment reads
    #[serde(default)]
    config: Option<Json>,
    // the same document as its JSON text, for a case whose number literals must reach the parser as written
    #[serde(default)]
    config_text: Option<String>,
    #[serde(default)]
    messages: Option<Vec<Message>>,
    // litellm_params.quality_router_default_model, which init_quality_router_deployment resolves
    #[serde(default)]
    default_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Fields {
    model: Option<String>,
    decision: Option<Json>,
    metadata: Option<Json>,
    headers: Json,
    warnings: Vec<String>,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let payload = &input.0;
    let settings = read_settings(payload)?;
    let service = service(&settings, payload.default_model.as_deref())?;
    let mut metadata = None;
    let routed = service
        .pre_route(payload.messages.as_deref(), &mut metadata)
        .map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    let (model, decision) = routed_fields(routed)?;
    Ok(ProbeOutput::QualityRouter(Output(Box::new(Fields {
        model,
        decision,
        headers: headers(metadata.as_ref())?,
        metadata,
        warnings: service.warnings().to_vec(),
    }))))
}

fn service(
    settings: &section::Settings,
    default_model: Option<&str>,
) -> Result<QualityRouter, OperationError> {
    let Some(router) = settings
        .deployments
        .iter()
        .find(|deployment| deployment.config.is_some())
    else {
        return Err(OperationError::InvalidRequest {
            message: "the config declares no quality_router_config".to_owned(),
        });
    };
    QualityRouter::new(
        &router.model_name,
        router.config.as_ref(),
        default_model,
        &settings.deployments,
    )
    .map_err(|error| OperationError::InvalidRequest {
        message: error.to_string(),
    })
}

fn routed_fields(
    routed: Option<Route<'_>>,
) -> Result<(Option<String>, Option<Json>), OperationError> {
    let Some(route) = routed else {
        return Ok((None, None));
    };
    let decision = route
        .decision
        .record()
        .map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    Ok((Some(route.model), Some(decision)))
}

// the section's typed view reads the document the same way a config file is read, environment and all
fn read_settings(payload: &Payload) -> Result<section::Settings, OperationError> {
    let source = match (&payload.config, &payload.config_text) {
        (_, Some(text)) => text.clone(),
        (Some(document), None) => {
            serde_json::to_string(document).map_err(|source| OperationError::InvalidRequest {
                message: source.to_string(),
            })?
        }
        (None, None) => {
            return Err(OperationError::InvalidRequest {
                message: "the input declares no config document".to_owned(),
            });
        }
    };
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&source)
    .map(|config| config.sections.quality_router)
    .map_err(|error| OperationError::InvalidRequest {
        message: error.to_string(),
    })
}

// the metadata a response header helper reads, wrapped the way the router passes the request kwargs
fn headers(metadata: Option<&Json>) -> Result<Json, OperationError> {
    let mut fields = IndexMap::new();
    fields.insert(
        "metadata".to_owned(),
        metadata.cloned().unwrap_or(Json::Null),
    );
    let request = serde_json::to_value(Json::Object(fields)).map_err(|source| {
        OperationError::InvalidRequest {
            message: source.to_string(),
        }
    })?;
    let routing: RoutingValue =
        serde_json::from_value(request).map_err(|source| OperationError::InvalidRequest {
            message: source.to_string(),
        })?;
    let mut additional = BTreeMap::new();
    apply_quality_headers(&mut additional, &routing);
    let value =
        serde_json::to_value(additional).map_err(|source| OperationError::InvalidRequest {
            message: source.to_string(),
        })?;
    serde_json::from_value(value).map_err(|source| OperationError::InvalidRequest {
        message: source.to_string(),
    })
}
