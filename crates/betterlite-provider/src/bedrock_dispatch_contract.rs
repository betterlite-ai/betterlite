use std::collections::BTreeMap;

use betterlite_bedrock::aws_endpoints::error::AwsEndpointError;
use betterlite_bedrock::aws_endpoints::model_id::bedrock_model_id;
use betterlite_bedrock::aws_endpoints::provider::invoke_provider;
use betterlite_bedrock::aws_endpoints::region::{LlmRegion, llm_region_name};
use betterlite_bedrock::aws_endpoints::runtime::{
    BedrockEndpointType, RuntimeEndpointInput, runtime_endpoint,
};
use betterlite_bedrock::model_routes::{BedrockRoute, ModelRouteSelector, ParamMap, RouteError};
use indexmap::IndexSet;
use serde_json::value::RawValue;
use thiserror::Error;

use crate::provider::Adapter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bedrock;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BedrockRequest;

pub struct BedrockResult;

// The plan a Bedrock call runs with: the route B598 selected, the id and region B591 resolved, and the runtime endpoint
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BedrockPrepared {
    pub operation: BedrockRoute,
    pub model: String,
    pub model_id: String,
    pub region: String,
    pub endpoint_url: String,
    pub proxy_endpoint_url: String,
}

// AWS values B591's config section resolved before the request, in the order _get_aws_region_name reads them
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BedrockAwsSettings {
    pub aws_region_name: Option<String>,
    pub aws_region: Option<String>,
    pub session_region: Option<String>,
    pub aws_bedrock_runtime_endpoint: Option<String>,
}

// Everything a plan reads, already normalized: the request parameters, the caller's Bedrock catalog and the settings
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BedrockPlanRequest {
    pub model: String,
    pub model_id: Option<String>,
    pub aws_region_name: Option<String>,
    pub api_base: Option<String>,
    pub aws_bedrock_runtime_endpoint: Option<String>,
    pub settings: BedrockAwsSettings,
    pub request_params: ParamMap,
    pub additional_drop_params: Vec<String>,
    pub price_map: ParamMap,
    pub converse_models: IndexSet<String>,
    pub request_metadata_owned: bool,
}

// The closed set of native Bedrock operations; a family answers unsupported until its own task installs its arm
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BedrockOperation {
    Plan(BedrockPlanRequest),
    Chat,
    Other,
    Services,
    Image,
    Agent,
    Interactive,
}

#[derive(Debug, Error)]
pub enum BedrockDispatchError {
    #[error(transparent)]
    Route(#[from] RouteError),
    #[error(transparent)]
    Endpoint(#[from] AwsEndpointError),
    #[error("the model id cannot be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("betterlite does not run the {operation} operation through the bedrock provider")]
    Unsupported { operation: &'static str },
}

// No SDK and no chat body enters here: the dispatcher only composes the route and endpoint leaves
pub fn dispatch(operation: &BedrockOperation) -> Result<BedrockPrepared, BedrockDispatchError> {
    match operation {
        BedrockOperation::Plan(request) => plan(request),
        BedrockOperation::Chat => Err(BedrockDispatchError::Unsupported { operation: "chat" }),
        BedrockOperation::Other => Err(BedrockDispatchError::Unsupported {
            operation: "other endpoints",
        }),
        BedrockOperation::Services => Err(BedrockDispatchError::Unsupported {
            operation: "services",
        }),
        BedrockOperation::Image => Err(BedrockDispatchError::Unsupported {
            operation: "image generation",
        }),
        BedrockOperation::Agent => Err(BedrockDispatchError::Unsupported { operation: "agent" }),
        BedrockOperation::Interactive => Err(BedrockDispatchError::Unsupported {
            operation: "interactive",
        }),
    }
}

fn plan(request: &BedrockPlanRequest) -> Result<BedrockPrepared, BedrockDispatchError> {
    let selector = ModelRouteSelector::new(
        &request.price_map,
        &request.converse_models,
        request.request_metadata_owned,
    )?;
    let operation = selector.route_for_request(
        &request.model,
        &request.request_params,
        Some(&request.additional_drop_params),
    )?;
    let provider = invoke_provider(&request.model);
    let mut optional_params: BTreeMap<String, Box<RawValue>> = BTreeMap::new();
    if let Some(model_id) = &request.model_id {
        let raw_id = serde_json::value::to_raw_value(model_id)
            .map_err(|source| BedrockDispatchError::Encode { source })?;
        optional_params.insert(String::from("model_id"), raw_id);
    }
    let model_id = bedrock_model_id(&mut optional_params, provider, &request.model)?;
    // the Converse handler passes the caller's model_id override to the region resolver; every other route does not
    let model_id_region = match operation {
        BedrockRoute::Converse => request.model_id.as_deref(),
        _ => None,
    };
    let region = llm_region_name(LlmRegion {
        explicit: request.aws_region_name.as_deref(),
        model: Some(&request.model),
        model_id: model_id_region,
        environment_region_name: request.settings.aws_region_name.as_deref(),
        environment_region: request.settings.aws_region.as_deref(),
        session_region: request.settings.session_region.as_deref(),
    })?;
    let endpoint = runtime_endpoint(RuntimeEndpointInput {
        api_base: request.api_base.as_deref(),
        configured_endpoint: request.aws_bedrock_runtime_endpoint.as_deref(),
        environment_endpoint: request.settings.aws_bedrock_runtime_endpoint.as_deref(),
        region: &region,
        endpoint_type: BedrockEndpointType::Runtime,
    });
    Ok(BedrockPrepared {
        operation,
        model: request.model.clone(),
        model_id,
        region,
        endpoint_url: endpoint.endpoint_url,
        proxy_endpoint_url: endpoint.proxy_endpoint_url,
    })
}

impl Adapter for Bedrock {
    const NAME: &'static str = "bedrock";
}

mod tests;
