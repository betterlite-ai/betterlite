use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_http::user_url::validate::Resolve;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use serde::Serialize;

use crate::agentcore_stream::IdSource;
use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::converse_options::supported::supported_openai_params as converse_supported;
use crate::converse_params::compose::{
    ConverseParamsRequest, map_openai_params as converse_map_openai_params,
};
use crate::converse_params::error::ConverseParamsError;
use crate::converse_request::{
    ConverseMedia, ConverseRequestBody, ConverseRequestInput, converse_request,
};
use crate::converse_response::request::RawReply;
use crate::converse_response::transform::{
    TransformCall, TransformError, transform_response as converse_transform_response,
};
use crate::converse_response::wire::ModelResponse;
use crate::converse_usage::transform::ReasoningTokenCounter;
use crate::model_routes::{ParamMap, ParamValue};

use super::error::NovaChatError;

const CACHE_POINT_FIELD: &str = "cachePoint";
const CONTENT_FIELD: &str = "content";
const INJECTION_POINTS_FIELD: &str = "cache_control_injection_points";
const LOCATION_FIELD: &str = "location";
const MESSAGES_FIELD: &str = "messages";
const ROLE_FIELD: &str = "role";
const SOFTWARE_FIELD: &str = "system";
const TEXT_FIELD: &str = "text";
const TOOL_CONFIG_LOCATION: &str = "tool_config";

// BedrockInvokeNovaRequest: the only five members /bedrock/invoke/ accepts for Nova.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InvokeNovaBody {
    pub messages: Vec<Object>,
    #[serde(rename = "inferenceConfig")]
    pub inference_config: Object,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub system: Vec<Object>,
    #[serde(rename = "toolConfig", skip_serializing_if = "Option::is_none")]
    pub tool_config: Option<Json>,
    #[serde(rename = "guardrailConfig", skip_serializing_if = "Option::is_none")]
    pub guardrail_config: Option<Json>,
}

pub struct InvokeNovaOutcome {
    pub body: InvokeNovaBody,
    pub tool_name_mappings: ToolNameMap,
}

// _inline_cache_points reads both lists with a default, so a missing one inlines to an empty list.
pub struct InlineRequest<'request> {
    pub system: &'request [Json],
    pub messages: &'request [Json],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InlineBody {
    pub system: Vec<Object>,
    pub messages: Vec<Object>,
}

pub async fn transform_request<Source, Transport, Lookup, R, Ids>(
    request: ConverseRequestInput<'_, Lookup>,
    media: ConverseMedia<'_, R, Ids>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<InvokeNovaOutcome, NovaChatError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
    R: Resolve,
    Ids: Fn() -> String + Sync,
{
    let mut filtered = without_tool_config_points(request.optional_params);
    let input = ConverseRequestInput {
        model: request.model,
        messages: request.messages,
        optional_params: &mut filtered,
        litellm_params: request.litellm_params,
        headers: request.headers,
        settings: request.settings,
        model_cost: request.model_cost,
        catalogs: request.catalogs,
        lookup: request.lookup,
        packaged: request.packaged,
    };
    let outcome = converse_request(input, media, environment).await?;
    Ok(InvokeNovaOutcome {
        body: transform_converse(&outcome.body)?,
        tool_name_mappings: outcome.tool_name_mappings,
    })
}

// _inline_cache_points, _remove_empty_system_messages and _filter_allowed_fields in one pass.
#[must_use]
pub fn transform_converse(body: &ConverseRequestBody) -> Result<InvokeNovaBody, NovaChatError> {
    let inline = inline_cache_points(&InlineRequest {
        system: &body.common.system,
        messages: &body.messages,
    })?;
    Ok(InvokeNovaBody {
        messages: inline.messages,
        inference_config: body.common.inference_config.clone(),
        system: inline.system,
        tool_config: body.common.tool_config.clone(),
        guardrail_config: body.common.guardrail_config.clone(),
    })
}

pub fn inline_cache_points(request: &InlineRequest<'_>) -> Result<InlineBody, NovaChatError> {
    let system = block_objects(request.system, SOFTWARE_FIELD)?;
    let mut messages = Vec::with_capacity(request.messages.len());
    for message in request.messages {
        messages.push(inline_message(message)?);
    }
    Ok(InlineBody {
        system: inline_block_cache_points(&system),
        messages,
    })
}

// A message keeps only role and content, and its content gains the points of the Converse body.
fn inline_message(message: &Json) -> Result<Object, NovaChatError> {
    let Some(fields) = message.as_object() else {
        return Err(NovaChatError::BodyField {
            field: MESSAGES_FIELD,
            expected: "an object",
        });
    };
    let Some(role) = fields.get(ROLE_FIELD) else {
        return Err(NovaChatError::BodyField {
            field: ROLE_FIELD,
            expected: "a member of every message",
        });
    };
    let Some(Json::List(content)) = fields.get(CONTENT_FIELD) else {
        return Err(NovaChatError::BodyField {
            field: CONTENT_FIELD,
            expected: "a list of blocks",
        });
    };
    let blocks = inline_block_cache_points(&block_objects(content, CONTENT_FIELD)?);
    let fields: Object = [
        (ROLE_FIELD.to_owned(), role.clone()),
        (
            CONTENT_FIELD.to_owned(),
            Json::List(blocks.into_iter().map(Json::Object).collect()),
        ),
    ]
    .into_iter()
    .collect();
    Ok(fields)
}

// A standalone point moves back to the last text block, across any image, toolUse or toolResult.
#[must_use]
pub fn inline_block_cache_points(blocks: &[Object]) -> Vec<Object> {
    let mut inlined: Vec<Object> = Vec::with_capacity(blocks.len());
    for block in blocks {
        let Some(point) = standalone_cache_point(block) else {
            inlined.push(block.clone());
            continue;
        };
        let Some(anchor) = inlined
            .iter()
            .rposition(|prior| prior.contains_key(TEXT_FIELD))
        else {
            continue;
        };
        let Some(target) = inlined.get_mut(anchor) else {
            continue;
        };
        target.insert(CACHE_POINT_FIELD.to_owned(), point.clone());
    }
    inlined
}

fn standalone_cache_point(block: &Object) -> Option<&Json> {
    if block.len() != 1 {
        return None;
    }
    match block.get(CACHE_POINT_FIELD) {
        Some(Json::Null) | None => None,
        Some(point) => Some(point),
    }
}

fn block_objects(blocks: &[Json], field: &'static str) -> Result<Vec<Object>, NovaChatError> {
    let mut objects = Vec::with_capacity(blocks.len());
    for block in blocks {
        let Some(fields) = block.as_object() else {
            return Err(NovaChatError::BodyField {
                field,
                expected: "a list of objects",
            });
        };
        objects.push(fields.clone());
    }
    Ok(objects)
}

// InvokeModel has no tool caching, so a tool_config point never reaches the Converse transform.
#[must_use]
pub fn without_tool_config_points(params: &ParamMap) -> ParamMap {
    let Some(ParamValue::List(points)) = params.get(INJECTION_POINTS_FIELD) else {
        return params.clone();
    };
    if !points
        .iter()
        .all(|point| matches!(point, ParamValue::Mapping(_)))
    {
        return params.clone();
    }
    let kept = points
        .iter()
        .filter(|point| !is_tool_config_point(point))
        .cloned()
        .collect();
    let mut filtered = params.clone();
    filtered.insert(INJECTION_POINTS_FIELD.to_owned(), ParamValue::List(kept));
    filtered
}

fn is_tool_config_point(point: &ParamValue) -> bool {
    let ParamValue::Mapping(fields) = point else {
        return false;
    };
    matches!(
        fields.get(LOCATION_FIELD),
        Some(ParamValue::Text(location)) if location == TOOL_CONFIG_LOCATION
    )
}

// LiteLLM's AmazonInvokeNovaConfig.get_supported_openai_params hands the model to AmazonConverseConfig.
pub async fn supported_openai_params<Source, Transport>(
    model: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> Vec<String>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    converse_supported(model, environment, catalogs).await
}

// LiteLLM's AmazonInvokeNovaConfig.map_openai_params passes the request to AmazonConverseConfig.
pub async fn map_openai_params<Source, Transport, Lookup>(
    request: &ConverseParamsRequest<'_, Lookup>,
    optional_params: &mut ParamMap,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    names: &mut ToolNameMap,
) -> Result<(), ConverseParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    converse_map_openai_params(
        // Nova adds no parameter of its own; the Converse mapping is the whole behavior
        request,
        optional_params,
        environment,
        names,
    )
    .await
}

// LiteLLM's AmazonInvokeNovaConfig.transform_response passes every argument to AmazonConverseConfig.
pub fn transform_response<Counter>(
    call: TransformCall<'_, '_, '_>,
    reply: &RawReply<'_>,
    model_response: ModelResponse,
    ids: &dyn IdSource,
    counter: &Counter,
) -> Result<ModelResponse, TransformError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    converse_transform_response(
        // the parent owns message conversion, usage and the error mapping this invoker keeps
        call,
        reply,
        model_response,
        ids,
        counter,
    )
}
