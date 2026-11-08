use betterlite_mcp::wire_compat::errors::{OpenApiUpstreamError, ToolResultError};
use betterlite_mcp::wire_compat::result_conversion::to_gateway_tool;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

use capabilities::{AdvertisedFields, DiscoveryFields, PolicyFields};
use conversion::{BodyFields, Conversion, ConvertFields, GatewayToolFields, ProtocolFields};
use errors::{OpenApiFields, ServerListFields, UpstreamAuthFields};

mod capabilities;
mod conversion;
mod errors;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Discovery(fields) => Ok(capabilities::run_discovery(fields)),
            Input::VersionPolicy(fields) => capabilities::run_policy(fields),
            Input::AdvertisedVersions(fields) => capabilities::run_advertised(fields),
            Input::WireCompatFor(fields) => Ok(conversion::run_wire_compat(fields)),
            Input::ParseHttpBody(fields) => conversion::run_parse_body(fields),
            Input::ToCallToolResult(fields) => {
                Ok(conversion::run_convert(fields, &Conversion::Wire))
            }
            Input::CompleteCallToolResult(fields) => {
                Ok(conversion::run_convert(fields, &Conversion::Complete))
            }
            Input::ToGatewayTool(fields) => Ok(json!(to_gateway_tool(&fields.tool, &fields.name))),
            Input::UrlCredentials => Ok(errors::run_url_credentials()),
            Input::UpstreamAuth(fields) => Ok(errors::run_upstream_auth(fields)),
            Input::OpenapiUpstream(fields) => {
                let error = OpenApiUpstreamError {
                    status_code: fields.status_code,
                    server_name: fields.server_name.clone(),
                };
                Ok(json!({
                    "message": error.to_string(),
                    "status_code": error.status_code,
                    "server_name": error.server_name,
                }))
            }
            Input::ToolResultError => Ok(json!({ "message": ToolResultError.to_string() })),
            Input::ServerListError(fields) => Ok(errors::run_server_list(fields)),
        }
    }
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Discovery(DiscoveryFields),
    VersionPolicy(PolicyFields),
    AdvertisedVersions(AdvertisedFields),
    WireCompatFor(ProtocolFields),
    ParseHttpBody(BodyFields),
    ToCallToolResult(ConvertFields),
    CompleteCallToolResult(ConvertFields),
    ToGatewayTool(GatewayToolFields),
    UrlCredentials,
    UpstreamAuth(UpstreamAuthFields),
    OpenapiUpstream(OpenApiFields),
    ToolResultError,
    ServerListError(ServerListFields),
}
