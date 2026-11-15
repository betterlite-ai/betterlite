use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_openai::base_helpers::append_tools;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;

use crate::anthropic_capability_catalog::capability::supports_model_capability;
use crate::anthropic_capability_catalog::forced_tool::{
    apply_forced_tool_choice, forced_tool_use_unsupported,
};
use crate::option_primitives::{WebSearchOptions, map_web_search_tool};
use crate::output_schema::mapping::{ResponseFormat, map_output, map_tool};
use crate::params::ParamsRequest;
use crate::params::error::ParamsError;
use crate::params::value;
use crate::tools::{map_tool_choice, map_tools};

pub const RESPONSE_FORMAT_TOOL_NAME: &str = "json_tool_call";

// The base helper owns the append-or-concatenate rule that every tools branch goes through
fn append_params<T>(
    optional_params: &mut IndexMap<String, Json>,
    tools: Vec<T>,
) -> Result<(), ParamsError>
where
    T: serde::Serialize,
{
    let mut base = value::to_wire(optional_params)?;
    let encoded = tools
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(ParamsError::Encode)?;
    append_tools(&mut base, encoded)?;
    let rewritten = serde_json::to_value(base).map_err(ParamsError::Encode)?;
    *optional_params = serde_json::from_value(rewritten).map_err(ParamsError::Decode)?;
    Ok(())
}

pub fn apply_tools(
    optional_params: &mut IndexMap<String, Json>,
    value: &Json,
) -> Result<(), ParamsError> {
    let Json::List(items) = value else {
        return Err(ParamsError::NotList { field: "tools" });
    };
    let mapped: Vec<PromptJson> = items.iter().map(value::prompt_json).collect();
    let mapping = map_tools(&mapped)?;
    append_params(optional_params, mapping.tools)?;
    if !mapping.mcp_servers.is_empty() {
        optional_params.insert(
            "mcp_servers".to_owned(),
            value::encode(&mapping.mcp_servers)?,
        );
    }
    Ok(())
}

const fn parallel_flag(value: Option<&Json>) -> Result<Option<bool>, ParamsError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(ParamsError::NotBool {
            field: "parallel_tool_calls",
        }),
    }
}

// Either parameter reads both from the request, so the second pass maps the same pair again
pub fn apply_tool_choice(
    request: &ParamsRequest<'_>,
    non_default_params: &IndexMap<String, Json>,
    optional_params: &mut IndexMap<String, Json>,
) -> Result<(), ParamsError> {
    let choice = non_default_params
        .get("tool_choice")
        .map(value::prompt_json);
    let parallel = parallel_flag(non_default_params.get("parallel_tool_calls"))?;
    let Some(chosen) = map_tool_choice(choice.as_ref(), parallel)? else {
        return Ok(());
    };
    let encoded_choice = value::encode(&chosen)?;
    let forced = apply_forced_tool_choice(
        request.model,
        &encoded_choice,
        request.drop,
        request.catalogs,
    )?;
    optional_params.insert("tool_choice".to_owned(), forced);
    Ok(())
}

pub async fn apply_response_format<Source, Transport>(
    request: &ParamsRequest<'_>,
    optional_params: &mut IndexMap<String, Json>,
    value: &Json,
    thinking_enabled: bool,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<(), ParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    if value::as_object(value).is_none() {
        return Ok(());
    }
    let format: ResponseFormat =
        serde_json::from_value(serde_json::to_value(value).map_err(ParamsError::Encode)?)
            .map_err(ParamsError::Decode)?;
    let native = supports_model_capability(
        request.model,
        "supports_native_structured_output",
        request.provider,
        environment,
        request.catalogs,
    )
    .await;
    if native {
        if let Some(output) = map_output(Some(&format))? {
            optional_params.insert("output_format".to_owned(), value::encode(&output)?);
        }
        optional_params.insert("json_mode".to_owned(), Json::Bool(true));
        return Ok(());
    }
    let Some(tool) = map_tool(Some(&format))? else {
        return Ok(());
    };
    if !thinking_enabled && !forced_tool_use_unsupported(request.model, request.catalogs) {
        let mut choice = IndexMap::new();
        choice.insert(
            "name".to_owned(),
            Json::Text(RESPONSE_FORMAT_TOOL_NAME.to_owned()),
        );
        choice.insert("type".to_owned(), Json::Text("tool".to_owned()));
        optional_params.insert("tool_choice".to_owned(), Json::Object(choice));
    }
    append_params(optional_params, vec![tool])?;
    optional_params.insert("json_mode".to_owned(), Json::Bool(true));
    Ok(())
}

pub fn apply_web_search(
    optional_params: &mut IndexMap<String, Json>,
    value: &Json,
) -> Result<(), ParamsError> {
    if value::as_object(value).is_none() {
        return Ok(());
    }
    let options: WebSearchOptions =
        serde_json::from_value(serde_json::to_value(value).map_err(ParamsError::Encode)?)
            .map_err(ParamsError::Decode)?;
    append_params(optional_params, vec![map_web_search_tool(&options)?])
}
