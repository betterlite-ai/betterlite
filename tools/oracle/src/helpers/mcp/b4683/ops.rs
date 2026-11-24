use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_mcp::credential_contracts::errors::CredError;
use betterlite_mcp::openapi_tools::base_url::get_base_url;
use betterlite_mcp::openapi_tools::document::{
    Components, Operation, Parameter, PathItem, parse_openapi_document,
};
use betterlite_mcp::openapi_tools::input_schema::build_input_schema;
use betterlite_mcp::openapi_tools::output::OpenApiToolOutput;
use betterlite_mcp::openapi_tools::parameters::{extract_parameters, resolve_operation_params};
use betterlite_mcp::openapi_tools::request_headers::{OpenApiCallContext, merge_request_headers};
use betterlite_mcp::openapi_tools::tool::{OpenApiTool, OpenApiToolConfig, OpenApiToolError};
use betterlite_mcp::openapi_tools::transport::{
    OpenApiRequest, OpenApiTransport, OpenApiTransportError,
};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::input::{
    AuthKind, BaseUrlInput, CallInput, MergeHeadersInput, OperationInput, Reply, ResolveParamsInput,
};

pub(crate) fn base_url(input: &BaseUrlInput) -> HelperResult {
    let bytes = serde_json::to_vec(&input.spec)?;
    let document = parse_openapi_document(&bytes).or_value_error()?;
    Ok(json!({"url": get_base_url(&document, input.spec_path.as_deref())}))
}

pub(crate) fn resolve_params(input: &ResolveParamsInput) -> HelperResult {
    let operation: Operation = serde_json::from_value(input.spec_operation.clone())?;
    let path_item: PathItem = serde_json::from_value(input.path_item.clone())?;
    let components: Components = serde_json::from_value(input.components.clone())?;
    let resolved = resolve_operation_params(&operation, &path_item, &components);
    let parameters: Vec<Value> = resolved.parameters.iter().map(parameter_json).collect();
    Ok(json!({"parameters": parameters}))
}

pub(crate) fn extract_params(input: &OperationInput) -> HelperResult {
    let operation: Operation = serde_json::from_value(input.spec_operation.clone())?;
    let extracted = extract_parameters(&operation);
    Ok(json!({
        "path": extracted.path,
        "query": extracted.query,
        "body": extracted.body,
    }))
}

pub(crate) fn input_schema(input: &OperationInput) -> HelperResult {
    let operation: Operation = serde_json::from_value(input.spec_operation.clone())?;
    let schema = build_input_schema(&operation);
    Ok(json!(schema))
}

pub(crate) fn merge_headers(input: &MergeHeadersInput) -> Value {
    let context = OpenApiCallContext {
        auth_header: input.auth_header.clone(),
        extra_headers: input.extra_headers.clone(),
        resolved_auth_headers: input.resolved_auth_headers.clone(),
    };
    let merged = merge_request_headers(&input.static_headers, &context);
    json!({"headers": merged})
}

pub(crate) fn call(input: &CallInput) -> HelperResult {
    let operation: Operation = serde_json::from_value(input.spec_operation.clone())?;
    let arguments: Object = serde_json::from_value(input.arguments.clone())?;
    let tool = OpenApiTool::new(OpenApiToolConfig {
        path: input.path.clone(),
        method: input.method.clone(),
        operation,
        base_url: input.base_url.clone(),
        headers: input.headers.clone(),
        server_label: input.server_label.clone(),
        relays_upstream_auth: input.relays_upstream_auth,
        auth_type: input.auth_type.map(AuthKind::mode),
        upstream_token_header: input.upstream_token_header.clone(),
    });
    let context = OpenApiCallContext {
        auth_header: input.auth_header.clone(),
        extra_headers: input.extra_headers.clone(),
        resolved_auth_headers: input.resolved_auth_headers.clone(),
    };
    let transport = PlannedTransport::new(&input.reply)?;
    let runtime = Builder::new_current_thread().enable_all().build()?;
    let outcome = runtime.block_on(tool.call(&arguments, &context, &transport));
    let request = transport.take().map(|sent| request_json(&sent));
    let (result, error) = match outcome {
        Ok(output) => (Some(output_json(output)), None),
        Err(failure) => (None, Some(error_json(&failure))),
    };
    Ok(json!({"request": request, "result": result, "error": error}))
}

fn parameter_json(parameter: &Parameter) -> Value {
    json!({
        "name": parameter.name,
        "in": parameter.location,
        "description": parameter.description,
        "required": parameter.required,
        "type": parameter.schema.as_ref().and_then(|schema| schema.kind.as_deref()),
    })
}

fn request_json(request: &OpenApiRequest) -> Value {
    let headers: BTreeMap<String, String> = request
        .headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect();
    let body = request
        .body
        .as_deref()
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok());
    json!({
        "method": request.method.as_str(),
        "url": request.url,
        "query": request.query,
        "headers": headers,
        "body": body,
    })
}

fn output_json(output: OpenApiToolOutput) -> Value {
    match output {
        OpenApiToolOutput::Text(text) => json!({"kind": "text", "text": text}),
        OpenApiToolOutput::Json {
            value,
            original_text,
        } => json!({"kind": "json", "text": original_text, "value": value}),
    }
}

fn error_json(error: &OpenApiToolError) -> Value {
    match error {
        OpenApiToolError::Credential(credential) => {
            json!({"kind": "credential", "status": credential_status(credential)})
        }
        OpenApiToolError::UpstreamAuth {
            status,
            www_authenticate,
            server,
        } => json!({
            "kind": "upstream_auth",
            "status": status,
            "www_authenticate": www_authenticate,
            "server": server,
        }),
        OpenApiToolError::UpstreamStatus { status, server } => json!({
            "kind": "upstream_status",
            "status": status,
            "server": server,
        }),
        OpenApiToolError::Body { .. }
        | OpenApiToolError::Transport(_)
        | OpenApiToolError::BodyText { .. } => json!({"kind": "other"}),
    }
}

const fn credential_status(error: &CredError) -> u16 {
    match error {
        CredError::Unauthorized(_) => 401,
        CredError::Misconfigured(_)
        | CredError::UrlCredentialsNotAllowed
        | CredError::UnsupportedMode(_) => 500,
        CredError::UpstreamUnavailable(_) => 503,
        CredError::PreconditionRequired(_) => 412,
        CredError::NotImplemented(_) => 501,
    }
}

struct PlannedTransport {
    status: u16,
    headers: HeaderMap,
    body: String,
    requests: Mutex<Vec<OpenApiRequest>>,
}

impl PlannedTransport {
    fn new(reply: &Reply) -> Result<Self, Box<HelperError>> {
        let mut headers = HeaderMap::new();
        for (name, value) in &reply.headers {
            let header_name = HeaderName::from_bytes(name.as_bytes()).or_value_error()?;
            let header_value = HeaderValue::from_str(value).or_value_error()?;
            headers.insert(header_name, header_value);
        }
        Ok(Self {
            status: reply.status,
            headers,
            body: reply.body.clone(),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn take(&self) -> Option<OpenApiRequest> {
        self.requests
            .lock()
            .ok()
            .and_then(|mut requests| requests.pop())
    }
}

impl OpenApiTransport for PlannedTransport {
    fn send<'call>(
        &'call self,
        request: OpenApiRequest,
    ) -> Pin<Box<dyn Future<Output = Result<UpstreamResponse, OpenApiTransportError>> + Send + 'call>>
    {
        Box::pin(async move {
            if let Ok(mut requests) = self.requests.lock() {
                requests.push(request);
            }
            Ok(UpstreamResponse {
                status: self.status,
                headers: self.headers.clone(),
                body: self.body.clone().into_bytes(),
            })
        })
    }
}
