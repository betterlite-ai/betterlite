use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::autorouter_facts::counts_as_present;
use crate::request_usage::payload::DiscoveryKwargs;

use super::DiscoveryError;

// standard_logging_object.metadata.mcp_tool_call_metadata: the call metadata a logged MCP tool call carries.
pub(super) fn mcp_call_metadata(
    kwargs: &DiscoveryKwargs,
) -> Result<Option<&JsonObject>, DiscoveryError> {
    let sl_object = match kwargs.standard_logging_object.as_ref() {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Object(object)) => object,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(DiscoveryError::NotAnObject {
                field: "standard_logging_object",
            });
        }
    };
    let Some(metadata_value) = sl_object
        .get("metadata")
        .filter(|value| counts_as_present(value))
    else {
        return Ok(None);
    };
    let Json::Object(metadata) = metadata_value else {
        return Err(DiscoveryError::NotAnObject {
            field: "standard_logging_object.metadata",
        });
    };
    let Some(mcp_value) = metadata
        .get("mcp_tool_call_metadata")
        .filter(|value| counts_as_present(value))
    else {
        return Ok(None);
    };
    let Json::Object(mcp) = mcp_value else {
        return Ok(None);
    };
    Ok(Some(mcp))
}
