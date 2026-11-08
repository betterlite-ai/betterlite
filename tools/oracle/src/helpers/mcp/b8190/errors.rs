use betterlite_mcp::wire_compat::errors::{
    ServerListError, UpstreamAuthError, url_credentials_exception,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpstreamAuthFields {
    status_code: u16,
    #[serde(default)]
    www_authenticate: Option<String>,
    server_name: String,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    request_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OpenApiFields {
    pub(super) status_code: u16,
    pub(super) server_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerListFields {
    fault: String,
    server_name: String,
}

pub(crate) fn run_url_credentials() -> Value {
    let exception = url_credentials_exception();
    json!({
        "status_code": exception.status_code,
        "detail": exception.detail,
        "headers": exception.headers,
    })
}

pub(crate) fn run_upstream_auth(fields: &UpstreamAuthFields) -> Value {
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

pub(crate) fn run_server_list(fields: &ServerListFields) -> Value {
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
