use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use http::Method;
use thiserror::Error;

use crate::client::sessions::factory::RedirectGuard;
use crate::credential_contracts::errors::CredError;
use crate::credentials::McpAuthType;
use crate::openapi_tools::document::Operation;
use crate::openapi_tools::output::{OpenApiToolOutput, parse_http_body};
use crate::openapi_tools::parameters::{ExtractedParameters, extract_parameters};
use crate::openapi_tools::path_values::sanitize_path_value;
use crate::openapi_tools::request_headers::{OpenApiCallContext, merge_request_headers};
use crate::openapi_tools::transport::{OpenApiRequest, OpenApiTransport, OpenApiTransportError};
use crate::openapi_tools::upstream;
use crate::static_credential::validate_static_credential;

pub const DEFAULT_AUTH_HEADER: &str = "Authorization";

#[derive(Debug, Clone)]
pub struct OpenApiToolConfig {
    pub path: String,
    pub method: String,
    pub operation: Operation,
    pub base_url: String,
    pub headers: BTreeMap<String, String>,
    pub server_label: Option<String>,
    pub relays_upstream_auth: bool,
    pub auth_type: Option<McpAuthType>,
    pub upstream_token_header: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OpenApiTool {
    path: String,
    method: String,
    base_url: String,
    headers: BTreeMap<String, String>,
    server_label: Option<String>,
    relays_upstream_auth: bool,
    auth_type: Option<McpAuthType>,
    upstream_token_header: Option<String>,
    params: ExtractedParameters,
}

#[derive(Debug, Error)]
pub enum OpenApiToolError {
    #[error(transparent)]
    Credential(CredError),
    #[error("the JSON request body cannot be encoded")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Transport(OpenApiTransportError),
    #[error("upstream server {server:?} answered with status {status}")]
    UpstreamStatus { status: u16, server: String },
    #[error("upstream server {server:?} answered with status {status} and a challenge")]
    UpstreamAuth {
        status: u16,
        www_authenticate: Option<String>,
        server: String,
    },
    #[error("the upstream response body is not UTF-8")]
    BodyText {
        #[source]
        source: std::string::FromUtf8Error,
    },
}

impl OpenApiTool {
    #[must_use]
    pub fn new(config: OpenApiToolConfig) -> Self {
        let params = extract_parameters(&config.operation);
        Self {
            method: config.method.to_lowercase(),
            path: config.path,
            base_url: config.base_url,
            headers: config.headers,
            server_label: config.server_label,
            relays_upstream_auth: config.relays_upstream_auth,
            auth_type: config.auth_type,
            upstream_token_header: config.upstream_token_header,
            params,
        }
    }

    // LiteLLM's tool_function: credential check, URL, query and body, one request, then classify and parse
    pub async fn call(
        &self,
        arguments: &Object,
        context: &OpenApiCallContext,
        transport: &dyn OpenApiTransport,
    ) -> Result<OpenApiToolOutput, OpenApiToolError> {
        let headers = self.effective_headers(context)?;
        let url = match self.url(arguments) {
            Ok(url) => url,
            Err(rejection) => return Ok(rejection),
        };
        let query = self.query(arguments);
        let body = self.json_body(arguments)?;
        let Some(method) = request_method(&self.method) else {
            return Ok(OpenApiToolOutput::Text(format!(
                "Unsupported HTTP method: {}",
                self.method
            )));
        };
        let sends_body = matches!(self.method.as_str(), "post" | "put" | "patch");
        let request = OpenApiRequest {
            method,
            redirect_guard: credential_guard(&url, &context.resolved_auth_headers),
            url,
            query,
            headers,
            body: if sends_body { body } else { None },
        };
        self.execute(request, transport).await
    }

    // LiteLLM checks the static credential before it builds the url or reads the arguments
    fn effective_headers(
        &self,
        context: &OpenApiCallContext,
    ) -> Result<BTreeMap<String, String>, OpenApiToolError> {
        let headers = merge_request_headers(&self.headers, context);
        if let Some(auth_type) = self.auth_type {
            let static_names: Vec<String> = self.headers.keys().cloned().collect();
            validate_static_credential(
                auth_type,
                &headers,
                self.upstream_token_header.as_deref(),
                &static_names,
            )
            .map_err(OpenApiToolError::Credential)?;
        }
        Ok(headers)
    }

    fn url(&self, arguments: &Object) -> Result<String, OpenApiToolOutput> {
        let mut url = format!("{}{}", self.base_url, self.path);
        for name in &self.params.path {
            let Some(value) = arguments.get(name) else {
                continue;
            };
            if !value.is_truthy() {
                continue;
            }
            let safe = match sanitize_path_value(value, name) {
                Ok(safe) => safe,
                Err(error) => {
                    return Err(OpenApiToolOutput::Text(format!(
                        "Invalid path parameter: {error}"
                    )));
                }
            };
            url = url.replace(&format!("{{{name}}}"), &safe);
            url = url.replace(&format!("{{{{{name}}}}}"), &safe);
        }
        Ok(url)
    }

    fn query(&self, arguments: &Object) -> Vec<(String, String)> {
        let mut query: Vec<(String, String)> = Vec::new();
        for name in &self.params.query {
            let Some(value) = arguments.get(name) else {
                continue;
            };
            if !value.is_truthy() {
                continue;
            }
            push_query(&mut query, name, value);
        }
        query
    }

    fn json_body(&self, arguments: &Object) -> Result<Option<Vec<u8>>, OpenApiToolError> {
        if self.params.body.is_empty() {
            return Ok(None);
        }
        let mut value = match arguments.get("body") {
            Some(argument) => argument.clone(),
            None => Json::Object(Object::new()),
        };
        if !value.is_truthy() {
            for name in &self.params.body {
                value = match arguments.get(name) {
                    Some(argument) => argument.clone(),
                    None => Json::Object(Object::new()),
                };
                if value.is_truthy() {
                    break;
                }
            }
        }
        let Some(payload) = body_payload(value) else {
            return Ok(None);
        };
        let bytes =
            serde_json::to_vec(&payload).map_err(|source| OpenApiToolError::Body { source })?;
        Ok(Some(bytes))
    }

    async fn execute(
        &self,
        request: OpenApiRequest,
        transport: &dyn OpenApiTransport,
    ) -> Result<OpenApiToolOutput, OpenApiToolError> {
        let server = self.server();
        let response = transport
            .send(request)
            .await
            .map_err(OpenApiToolError::Transport)?;
        upstream::classify(&response, self.relays_upstream_auth, server)?;
        let text = String::from_utf8(response.body)
            .map_err(|source| OpenApiToolError::BodyText { source })?;
        Ok(parse_http_body(&text))
    }

    fn server(&self) -> String {
        self.server_label
            .clone()
            .unwrap_or_else(|| format!("{} {}", self.method.to_uppercase(), self.path))
    }
}

fn body_payload(value: Json) -> Option<Json> {
    if matches!(value, Json::Object(_)) {
        return Some(value);
    }
    if !value.is_truthy() {
        return None;
    }
    match value {
        Json::Text(text) => parsed_body(&text),
        other => Some(data_wrapped(other)),
    }
}

fn parsed_body(text: &str) -> Option<Json> {
    match serde_json::from_str::<Json>(text) {
        Ok(Json::Null) => None,
        Ok(parsed) => Some(parsed),
        Err(_) => Some(data_wrapped(Json::Text(text.to_owned()))),
    }
}

fn data_wrapped(value: Json) -> Json {
    let mut fields = Object::new();
    fields.insert("data".to_owned(), value);
    Json::Object(fields)
}

fn push_query(query: &mut Vec<(String, String)>, name: &str, value: &Json) {
    if let Json::List(items) = value {
        for item in items {
            query.push((name.to_owned(), query_text(item)));
        }
        return;
    }
    query.push((name.to_owned(), query_text(value)));
}

fn query_text(value: &Json) -> String {
    match value {
        Json::Null => String::new(),
        Json::Bool(true) => "true".to_owned(),
        Json::Bool(false) => "false".to_owned(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => text.clone(),
        Json::List(_) | Json::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn request_method(method: &str) -> Option<Method> {
    match method {
        "get" => Some(Method::GET),
        "post" => Some(Method::POST),
        "put" => Some(Method::PUT),
        "delete" => Some(Method::DELETE),
        "patch" => Some(Method::PATCH),
        _ => None,
    }
}

fn credential_guard(url: &str, resolved: &BTreeMap<String, String>) -> Option<RedirectGuard> {
    let slot = resolved
        .keys()
        .find(|name| !name.eq_ignore_ascii_case(DEFAULT_AUTH_HEADER));
    RedirectGuard::new(url, slot.map(String::as_str))
}
