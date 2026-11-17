use betterlite_anthropic::anthropic_capability_catalog::{CatalogSources, DropParams};
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_openai::base_helpers::thinking_request;
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use indexmap::IndexMap;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::converse_options::family::is_nova_2_model;
use crate::converse_options::supported::auto_tool_choice;
use crate::converse_options::web_search::map_web_search_options;
use crate::converse_params::error::ConverseParamsError;
use crate::converse_params::inference::{apply_sampling, apply_stop, max_tokens_value};
use crate::converse_params::wire::transcode;
use crate::converse_params::{budget, mapping, merge, reasoning};
use crate::converse_tools::json_mode::{append_tools, apply_tool_call_transformation};
use crate::converse_tools::response_format::{ResponseFormatRequest, translate_response_format};
use crate::converse_tools::tool_choice::map_tool_choice_values;
use crate::model_routes::{ParamMap, ParamValue};

const CONTEXT_MANAGEMENT_FIELD: &str = "context_management";
const GPT_OSS_MARKER: &str = "gpt-oss";
const MAX_TOKENS_FIELD: &str = "maxTokens";
const PARALLEL_TOOL_USE_FIELD: &str = "_parallel_tool_use_config";
const STREAM_FIELD: &str = "stream";
const TOOL_CHOICE_FIELD: &str = "tool_choice";

pub struct ConverseParamsRequest<'request, Lookup> {
    pub model: &'request str,
    pub drop: DropParams,
    pub non_default_params: &'request ParamMap,
    pub catalogs: &'request CatalogSources<'request>,
    pub budgets: &'request Settings,
    pub lookup: &'request Lookup,
    pub packaged: &'request ModelCatalog,
}

// The dispatch branch LiteLLM runs over the request's params, in the caller's own order.
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
    for (param, value) in request.non_default_params {
        apply_mapped_param(request, optional_params, param, value, names)?;
        apply_reasoning_param(request, param, value, optional_params, environment).await?;
    }
    if !request.model.contains(GPT_OSS_MARKER) && !is_nova_2_model(request.model) {
        budget::update_thinking_budget(
            request.non_default_params,
            optional_params,
            request.budgets.bedrock_min,
        )?;
    }
    downgrade_forced_tool_choice(request.model, optional_params)
}

// Every parameter the loop maps without resolving a capability, in the arm LiteLLM gives it.
fn apply_mapped_param<Lookup>(
    request: &ConverseParamsRequest<'_, Lookup>,
    optional_params: &mut ParamMap,
    param: &str,
    value: &ParamValue,
    names: &mut ToolNameMap,
) -> Result<(), ConverseParamsError> {
    match param {
        "max_tokens" | "max_completion_tokens" => {
            let max_tokens = max_tokens_value(request.model, value);
            optional_params.insert(MAX_TOKENS_FIELD.to_owned(), max_tokens);
        }
        STREAM_FIELD => {
            optional_params.insert(STREAM_FIELD.to_owned(), value.clone());
        }
        "stop" => apply_stop(value, optional_params),
        "temperature" | "top_p" => apply_sampling(request, optional_params, param, value)?,
        "tools" => apply_tools(request, value, optional_params)?,
        "tool_choice" => apply_tool_choice(request, value, optional_params, names)?,
        "parallel_tool_calls" => {
            let config = merge::parallel_tool_use_config(value);
            optional_params.insert(
                PARALLEL_TOOL_USE_FIELD.to_owned(),
                ParamValue::Mapping(config),
            );
        }
        "context_management" => {
            if let Some(mapped) = mapping::context_management_value(value)? {
                optional_params.insert(CONTEXT_MANAGEMENT_FIELD.to_owned(), mapped);
            }
        }
        "requestMetadata" => mapping::map_request_metadata_param(value, optional_params)?,
        "service_tier" => {
            if let ParamValue::Text(tier) = value {
                mapping::map_service_tier_param(tier, optional_params);
            }
        }
        "web_search_options" if matches!(value, ParamValue::Mapping(_)) => {
            apply_web_search(request.model, optional_params)?;
        }
        _ => {}
    }
    Ok(())
}

// The arms whose prerequisites resolve capabilities, in the same elif chain's order.
async fn apply_reasoning_param<Source, Transport, Lookup>(
    request: &ConverseParamsRequest<'_, Lookup>,
    param: &str,
    value: &ParamValue,
    optional_params: &mut ParamMap,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), ConverseParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    if param == "response_format" && matches!(value, ParamValue::Mapping(_)) {
        return apply_response_format(request, optional_params, value, environment).await;
    }
    if param == "thinking" {
        return reasoning::apply_thinking(request, value, optional_params, environment).await;
    }
    if param == "reasoning_effort" {
        return reasoning::apply_reasoning_effort(request, value, optional_params, environment)
            .await;
    }
    if param == "output_config" && matches!(value, ParamValue::Mapping(_)) {
        return reasoning::apply_output_config(request, value, optional_params).await;
    }
    Ok(())
}

async fn apply_response_format<Source, Transport, Lookup>(
    request: &ConverseParamsRequest<'_, Lookup>,
    optional_params: &mut ParamMap,
    value: &ParamValue,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), ConverseParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let encoded: Json = transcode(value)?;
    let thinking_enabled = thinking_request(&transcode(request.non_default_params)?).thinking;
    let stream: Json = match request.non_default_params.get(STREAM_FIELD) {
        Some(stream) => transcode(stream)?,
        None => Json::Null,
    };
    let format = ResponseFormatRequest {
        model: request.model,
        is_thinking_enabled: thinking_enabled,
        stream: &stream,
        catalogs: request.catalogs,
    };
    let mut mapped: IndexMap<String, Json> = transcode(optional_params)?;
    translate_response_format(&encoded, &format, &mut mapped, environment).await?;
    *optional_params = transcode(&mapped)?;
    Ok(())
}

fn apply_tools<Lookup>(
    request: &ConverseParamsRequest<'_, Lookup>,
    value: &ParamValue,
    optional_params: &mut ParamMap,
) -> Result<(), ConverseParamsError> {
    let ParamValue::List(items) = value else {
        return Ok(());
    };
    let tools = items
        .iter()
        .map(transcode::<Json, ParamValue>)
        .collect::<Result<Vec<Json>, ConverseParamsError>>()?;
    let stream: Json = match request.non_default_params.get(STREAM_FIELD) {
        Some(stream) => transcode(stream)?,
        None => Json::Null,
    };
    let mut mapped: IndexMap<String, Json> = transcode(optional_params)?;
    apply_tool_call_transformation(&tools, request.model, &stream, &mut mapped)?;
    *optional_params = transcode(&mapped)?;
    Ok(())
}

fn apply_tool_choice<Lookup>(
    request: &ConverseParamsRequest<'_, Lookup>,
    value: &ParamValue,
    optional_params: &mut ParamMap,
    names: &mut ToolNameMap,
) -> Result<(), ConverseParamsError> {
    let encoded: Json = transcode(value)?;
    let mapped = map_tool_choice_values(
        request.model,
        &encoded,
        request.drop,
        request.catalogs,
        names,
    )?;
    if let Some(choice) = mapped {
        optional_params.insert(TOOL_CHOICE_FIELD.to_owned(), transcode(&choice)?);
    }
    Ok(())
}

fn apply_web_search(
    model: &str,
    optional_params: &mut ParamMap,
) -> Result<(), ConverseParamsError> {
    let Some(grounding) = map_web_search_options(model) else {
        return Ok(());
    };
    let mut mapped: IndexMap<String, Json> = transcode(optional_params)?;
    append_tools(&mut mapped, vec![transcode(&grounding)?])?;
    *optional_params = transcode(&mapped)?;
    Ok(())
}

// Reasoning cannot force a tool through Converse, so the whole choice block becomes auto.
fn downgrade_forced_tool_choice(
    model: &str,
    optional_params: &mut ParamMap,
) -> Result<(), ConverseParamsError> {
    if !thinking_request(&transcode(optional_params)?).thinking {
        return Ok(());
    }
    let forced = matches!(
        optional_params.get(TOOL_CHOICE_FIELD),
        Some(ParamValue::Mapping(fields)) if fields.contains_key("any") || fields.contains_key("tool")
    );
    if !forced {
        return Ok(());
    }
    tracing::info!(
        "{model} does not support forced tool use (tool_choice='required' or specific tool) when reasoning is enabled. Changing tool_choice to 'auto'."
    );
    optional_params.insert(
        TOOL_CHOICE_FIELD.to_owned(),
        ParamValue::Mapping(auto_tool_choice()),
    );
    Ok(())
}
