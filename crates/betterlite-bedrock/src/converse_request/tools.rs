use std::collections::BTreeMap;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_models::decisions::Json as DecisionsJson;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::bedrock_tools::capabilities::BedrockToolCapabilities;
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use betterlite_prompts::cache_defaults::attribution::record_gateway_injection;
use indexmap::IndexMap;

use crate::bedrock_converse_tools_beta::{ToolsAndBetaRequest, process_tools_and_beta};
use crate::converse_params::cache_point::build_cache_point_block;
use crate::converse_params::merge::drop_tool_choice_type;
use crate::converse_params::wire::transcode;
use crate::converse_request::body::PreparedRequest;
use crate::converse_request::error::ConverseRequestError;
use crate::event_frames::beta::BetaHeader;
use crate::model_routes::{ParamMap, ParamValue};

const CACHE_INJECTION_FIELD: &str = "cache_control_injection_points";
const CACHE_POINT_FIELD: &str = "cachePoint";
const CONTROL_FIELD: &str = "control";
const LOCATION_FIELD: &str = "location";
const TOOL_CHOICE_CAMEL_FIELD: &str = "toolChoice";
const TOOL_CHOICE_FIELD: &str = "tool_choice";
const TOOL_CONFIG_TOOLS_FIELD: &str = "tools";
const TOOLS_FIELD: &str = "tools";
const TOOL_CONFIG_LOCATION: &str = "tool_config";

pub(super) struct ToolsRequest<'request> {
    pub(super) model: &'request str,
    pub(super) model_cost: &'request ParamMap,
    pub(super) headers: Option<&'request BTreeMap<String, BetaHeader>>,
    pub(super) capabilities: BedrockToolCapabilities,
    pub(super) accepts_cache_points: bool,
}

pub(super) struct ToolsOutcome {
    pub(super) tool_config: Option<Json>,
    pub(super) names: ToolNameMap,
}

// The tools stage: convert through B683, append a tool_config cachePoint, then build toolConfig.
pub(super) async fn process<Source, Transport>(
    request: &ToolsRequest<'_>,
    prepared: &mut PreparedRequest,
    litellm_params: Option<&mut Object>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> Result<ToolsOutcome, ConverseRequestError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let original = original_tools(prepared.inference_params.shift_remove(TOOLS_FIELD))?;
    let mut params: IndexMap<String, DecisionsJson> = transcode(&prepared.additional)?;
    let tools_and_beta = process_tools_and_beta(
        ToolsAndBetaRequest {
            tools: &original,
            model: request.model,
            headers: request.headers,
            params: &mut params,
            capabilities: request.capabilities,
        },
        environment,
        catalogs,
    )
    .await?;
    prepared.additional = transcode(&params)?;

    let mut tools: Vec<Json> = tools_and_beta
        .tools
        .iter()
        .map(transcode::<Json, DecisionsJson>)
        .collect::<Result<_, _>>()?;
    append_injection(request, prepared, &mut tools, litellm_params)?;
    let tool_config = tool_config(prepared, tools)?;
    Ok(ToolsOutcome {
        tool_config,
        names: tools_and_beta.names,
    })
}

fn original_tools(value: Option<ParamValue>) -> Result<Vec<DecisionsJson>, ConverseRequestError> {
    match value {
        None | Some(ParamValue::Null) => Ok(Vec::new()),
        Some(ParamValue::List(items)) => items
            .iter()
            .map(transcode::<DecisionsJson, ParamValue>)
            .collect::<Result<_, _>>()
            .map_err(ConverseRequestError::from),
        Some(
            ParamValue::Bool(_)
            | ParamValue::Number(_)
            | ParamValue::Text(_)
            | ParamValue::Mapping(_),
        ) => Err(ConverseRequestError::FieldType {
            field: TOOLS_FIELD,
            expected: "a list",
        }),
    }
}

// Every point is consumed; only a tool_config point on a cache-capable model places a breakpoint.
fn append_injection(
    request: &ToolsRequest<'_>,
    prepared: &mut PreparedRequest,
    tools: &mut Vec<Json>,
    litellm_params: Option<&mut Object>,
) -> Result<(), ConverseRequestError> {
    let Some(points) = prepared.additional.shift_remove(CACHE_INJECTION_FIELD) else {
        return Ok(());
    };
    let ParamValue::List(points) = points else {
        if matches!(points, ParamValue::Null) {
            return Ok(());
        }
        return Err(ConverseRequestError::FieldType {
            field: CACHE_INJECTION_FIELD,
            expected: "a list",
        });
    };
    if points.is_empty() || tools.is_empty() || !request.accepts_cache_points {
        return Ok(());
    }
    for point in &points {
        let ParamValue::Mapping(fields) = point else {
            return Err(ConverseRequestError::FieldType {
                field: CACHE_INJECTION_FIELD,
                expected: "a list of objects",
            });
        };
        let is_tool_config = matches!(
            fields.get(LOCATION_FIELD),
            Some(ParamValue::Text(location)) if location == TOOL_CONFIG_LOCATION
        );
        if !is_tool_config {
            continue;
        }
        let block = build_cache_point_block(
            fields.get(CONTROL_FIELD),
            Some(request.model),
            request.model_cost,
        )?;
        let block: Json = transcode(&block)?;
        tools.push(Json::Object(Object::from_iter([(
            CACHE_POINT_FIELD.to_owned(),
            block,
        )])));
        // Spend attribution credits only a breakpoint this stage actually placed.
        if let Some(params) = litellm_params {
            record_gateway_injection(params, 1, false);
        }
        break;
    }
    Ok(())
}

// Without converted tools no toolConfig exists, and tool_choice stays in inferenceConfig.
fn tool_config(
    prepared: &mut PreparedRequest,
    tools: Vec<Json>,
) -> Result<Option<Json>, ConverseRequestError> {
    if tools.is_empty() {
        return Ok(None);
    }
    let choice = prepared
        .inference_params
        .shift_remove(TOOL_CHOICE_FIELD)
        .filter(|value| !matches!(value, ParamValue::Null));
    let mut fields = Object::from_iter([(TOOL_CONFIG_TOOLS_FIELD.to_owned(), Json::List(tools))]);
    if let Some(choice) = choice {
        let choice: Json = transcode(&choice)?;
        fields.insert(TOOL_CHOICE_CAMEL_FIELD.to_owned(), choice);
        drop_tool_choice_type(&mut prepared.additional);
    }
    Ok(Some(Json::Object(fields)))
}
