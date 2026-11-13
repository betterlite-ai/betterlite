use betterlite_mcp::wire_compat::errors::{
    OpenApiUpstreamError, ServerListError, ToolResultError, UpstreamAuthError,
    url_credentials_exception,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpstreamAuthFields {
    status_code: u16,
    www_authenticate: Option<String>,
    server_name: String,
    base_url: Option<String>,
    request_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ErrorInput {
    UrlCredentials,
    UpstreamAuth(UpstreamAuthFields),
    OpenapiUpstream(OpenApiUpstreamError),
    ToolResultError(ToolResultError),
    ServerListError(ServerListError<String>),
}

pub(crate) fn run(input: &ErrorInput) -> Value {
    match input {
        ErrorInput::UrlCredentials => json!(url_credentials_exception()),
        ErrorInput::UpstreamAuth(fields) => {
            let error = UpstreamAuthError {
                status_code: fields.status_code,
                www_authenticate: fields.www_authenticate.clone(),
                server_name: fields.server_name.clone(),
            };
            let exception =
                error.to_http_exception(fields.base_url.as_deref(), fields.request_path.as_deref());
            json!({
                "message": error.to_string(),
                "status_code": exception.status_code,
                "detail": exception.detail,
                "headers": exception.headers,
            })
        }
        ErrorInput::OpenapiUpstream(error) => json!({
            "message": error.to_string(),
            "status_code": error.status_code,
            "server_name": error.server_name,
        }),
        ErrorInput::ToolResultError(error) => json!({ "message": error.to_string() }),
        ErrorInput::ServerListError(error) => json!({
            "message": error.to_string(),
            "fault": error.fault,
            "server_name": error.server_name,
        }),
    }
}
