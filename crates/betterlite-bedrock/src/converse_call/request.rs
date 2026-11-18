use std::collections::BTreeMap;

use betterlite_http::user_url::validate::Resolve;
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::bedrock_tools::name::ToolNameMap;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::converse_content::ToolBlock;
use crate::converse_request::{ConverseRequestInput, converse_request};
use crate::converse_response::request::OptionalParams;
use crate::event_frames::beta::BetaHeader;
use crate::model_routes::{ParamMap, ParamValue};

use super::error::ConverseCallError;
use super::{ConverseAssembly, ConverseCall};

pub struct PreparedBody {
    pub data: String,
    pub tool_name_mappings: ToolNameMap,
    pub optional: OptionalParams,
}

// transform_request or _async_transform_request, whichever headers the route hands the config, then json.dumps' spacing and escaping over the typed body.
pub(crate) async fn build_body<Source, Transport, Lookup, R, Ids>(
    call: &mut ConverseCall<'_>,
    headers: Option<&BTreeMap<String, BetaHeader>>,
    assembly: ConverseAssembly<'_, '_, Source, Transport, Lookup, R, Ids>,
) -> Result<PreparedBody, ConverseCallError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
    R: Resolve,
    Ids: Fn() -> String + Sync,
{
    let outcome = converse_request(
        ConverseRequestInput {
            model: call.model,
            messages: call.messages,
            optional_params: &mut call.optional_params,
            litellm_params: Some(&mut call.litellm_params),
            headers,
            settings: call.settings,
            model_cost: assembly.model_cost,
            catalogs: assembly.catalogs,
            lookup: assembly.lookup,
            packaged: assembly.packaged,
        },
        assembly.media,
        assembly.environment,
    )
    .await?;
    let optional = reply_options(&call.optional_params)?;
    let value: Json = serde_json::from_str(&serde_json::to_string(&outcome.body)?)?;
    Ok(PreparedBody {
        data: python_json::dumps(&value),
        tool_name_mappings: outcome.tool_name_mappings,
        optional,
    })
}

// The reply conversion reads json_mode from the caller's options and the tools the json-mode filtering needs, both under the names request preparation left them.
fn reply_options(params: &ParamMap) -> Result<OptionalParams, ConverseCallError> {
    let json_mode = match params.get("json_mode") {
        None | Some(ParamValue::Null) => None,
        Some(ParamValue::Bool(value)) => Some(*value),
        Some(_) => return Err(ConverseCallError::Param { field: "json_mode" }),
    };
    let tools = match params.get("tools") {
        None | Some(ParamValue::Null) => None,
        Some(value) => {
            let encoded = serde_json::to_value(value)?;
            Some(
                serde_json::from_value::<Vec<ToolBlock>>(encoded)
                    .map_err(|_error| ConverseCallError::Param { field: "tools" })?,
            )
        }
    };
    Ok(OptionalParams { json_mode, tools })
}

#[path = "tests/request.rs"]
mod tests;
