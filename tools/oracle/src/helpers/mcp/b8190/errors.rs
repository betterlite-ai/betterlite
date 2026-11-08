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
#[serde(deny_unknown_fields)]
pub(crate) struct ServerFields {
    status_code: u16,
    server_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerListFields {
    fault: String,
    server_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ErrorInput {
    UrlCredentials,
    UpstreamAuth(UpstreamAuthFields),
    OpenapiUpstream(ServerFields),
    ToolResultError { message: String },
    ServerListError(ServerListFields),
}

pub(crate) fn run(input: &ErrorInput) -> Value {
    match input {
        ErrorInput::UrlCredentials => {
            let exception = url_credentials_exception();
            json!({
                "status_code": exception.status_code,
                "detail": exception.detail,
                "headers": exception.headers,
            })
        }
        ErrorInput::UpstreamAuth(fields) => upstream_auth(fields),
        ErrorInput::OpenapiUpstream(fields) => openapi_upstream(fields),
        ErrorInput::ToolResultError { message } => {
            let error = ToolResultError {
                message: message.clone(),
            };
            json!({ "message": error.to_string() })
        }
        ErrorInput::ServerListError(fields) => server_list(fields),
    }
}

fn upstream_auth(fields: &UpstreamAuthFields) -> Value {
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

fn openapi_upstream(fields: &ServerFields) -> Value {
    let error = OpenApiUpstreamError {
        status_code: fields.status_code,
        server_name: fields.server_name.clone(),
    };
    json!({
        "message": error.to_string(),
        "status_code": error.status_code,
        "server_name": error.server_name,
    })
}

fn server_list(fields: &ServerListFields) -> Value {
    let error = ServerListError {
        fault: fields.fault.clone(),
        server_name: fields.server_name.clone(),
    };
    json!({
        "message": error.to_string(),
        "fault": error.fault,
        "server_name": error.server_name,
    })
}
