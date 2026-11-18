use std::collections::BTreeMap;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_http::user_url::validate::Resolve;
use betterlite_logging::python_json;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use betterlite_prompts::message_content::Message;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::converse_content::ToolBlock;
use crate::converse_request::{
    ConverseMedia, ConverseRequestBody, ConverseRequestInput, ConverseSettings, converse_request,
};
use crate::converse_response::request::OptionalParams;
use crate::event_frames::beta::BetaHeader;
use crate::model_routes::{ParamMap, ParamValue};

use super::error::ConverseCallError;

pub struct BodyRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [Message],
    pub params: &'request mut ParamMap,
    pub litellm_params: &'request mut Object,
    pub headers: Option<&'request BTreeMap<String, BetaHeader>>,
    pub settings: ConverseSettings<'request>,
    pub model_cost: &'request ParamMap,
    pub catalogs: &'request CatalogSources<'request>,
    pub packaged: &'request ModelCatalog,
}

pub struct PreparedBody {
    pub data: String,
    pub tool_name_mappings: ToolNameMap,
    pub optional: OptionalParams,
}

// transform_request or _async_transform_request, whichever headers the route hands the config, then json.dumps' spacing and escaping over the typed body.
pub(crate) async fn build_body<Source, Transport, Lookup, R, Ids>(
    request: BodyRequest<'_>,
    lookup: &Lookup,
    media: ConverseMedia<'_, R, Ids>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<PreparedBody, ConverseCallError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
    R: Resolve,
    Ids: Fn() -> String + Sync,
{
    let BodyRequest {
        model,
        messages,
        params,
        litellm_params,
        headers,
        settings,
        model_cost,
        catalogs,
        packaged,
    } = request;
    let outcome = converse_request(
        ConverseRequestInput {
            model,
            messages,
            optional_params: &mut *params,
            litellm_params: Some(&mut *litellm_params),
            headers,
            settings,
            model_cost,
            catalogs,
            lookup,
            packaged,
        },
        media,
        environment,
    )
    .await?;
    let optional = reply_options(&*params)?;
    Ok(PreparedBody {
        data: body_text(&outcome.body)?,
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

fn body_text(body: &ConverseRequestBody) -> Result<String, ConverseCallError> {
    let value: Json = serde_json::from_str(&serde_json::to_string(body)?)?;
    Ok(python_json::dumps(&value))
}

#[path = "tests/request.rs"]
mod tests;
