use std::collections::BTreeMap;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_models::decisions::Json as DecisionsJson;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::bedrock_tools::capabilities::{BedrockToolCapabilities, CachePointSupport};
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use betterlite_prompts::cache_defaults::attribution::record_gateway_injection;
use indexmap::IndexMap;

use crate::bedrock_converse_tools_beta::{ToolsAndBetaRequest, process_tools_and_beta};
use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::bedrock_model_capabilities::strict_tools::converse_supports_strict_tools;
use crate::converse_params::cache_point::build_cache_point_block;
use crate::converse_params::merge::drop_tool_choice_type;
use crate::converse_params::wire::transcode;
use crate::converse_request::ConverseRequestInput;
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

pub(super) async fn process_tools<Source, Transport, Lookup>(
    input: &mut ConverseRequestInput<'_, Lookup>,
    prepared: &mut PreparedRequest,
    cache_points: CachePointSupport,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<ToolsOutcome, ConverseRequestError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let capabilities = BedrockToolCapabilities {
        cache_points,
        supports_strict_tools: converse_supports_strict_tools(
            input.model,
            input.lookup,
            input.packaged,
        )
        .await,
    };
    process(
        &ToolsRequest {
            model: input.model,
            model_cost: input.model_cost,
            headers: input.headers,
            capabilities,
            accepts_cache_points: !matches!(cache_points, CachePointSupport::Unsupported),
        },
        prepared,
        input.litellm_params.as_deref_mut(),
        environment,
        input.catalogs,
    )
    .await
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
    let ParamValue::List(entries) = points else {
        if matches!(points, ParamValue::Null) {
            return Ok(());
        }
        return Err(ConverseRequestError::FieldType {
            field: CACHE_INJECTION_FIELD,
            expected: "a list",
        });
    };
    if entries.is_empty() || tools.is_empty() || !request.accepts_cache_points {
        return Ok(());
    }
    place_point(request, &entries, tools, litellm_params)
}

fn place_point(
    request: &ToolsRequest<'_>,
    points: &[ParamValue],
    tools: &mut Vec<Json>,
    mut litellm_params: Option<&mut Object>,
) -> Result<(), ConverseRequestError> {
    for point in points {
        let ParamValue::Mapping(fields) = point else {
            return Err(ConverseRequestError::FieldType {
                field: CACHE_INJECTION_FIELD,
                expected: "a list of objects",
            });
        };
        if !tool_config_point(fields) {
            continue;
        }
        tools.push(cache_point(fields.get(CONTROL_FIELD), request)?);
        // Spend attribution credits only a breakpoint this stage actually placed.
        if let Some(params) = litellm_params.take() {
            record_gateway_injection(params, 1, false);
        }
        break;
    }
    Ok(())
}

fn tool_config_point(fields: &IndexMap<String, ParamValue>) -> bool {
    matches!(
        fields.get(LOCATION_FIELD),
        Some(ParamValue::Text(location)) if location == TOOL_CONFIG_LOCATION
    )
}

// B683's block for one breakpoint, wrapped as the cachePoint member the tool list carries.
fn cache_point(
    control: Option<&ParamValue>,
    request: &ToolsRequest<'_>,
) -> Result<Json, ConverseRequestError> {
    let block = build_cache_point_block(control, Some(request.model), request.model_cost)?;
    let encoded: Json = transcode(&block)?;
    Ok(Json::Object(Object::from_iter([(
        CACHE_POINT_FIELD.to_owned(),
        encoded,
    )])))
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
    if let Some(selected) = choice {
        let encoded: Json = transcode(&selected)?;
        fields.insert(TOOL_CHOICE_CAMEL_FIELD.to_owned(), encoded);
        drop_tool_choice_type(&mut prepared.additional);
    }
    Ok(Some(Json::Object(fields)))
}
