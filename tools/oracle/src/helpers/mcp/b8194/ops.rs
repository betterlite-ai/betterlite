use std::collections::BTreeMap;
use std::str::from_utf8;

use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_mcp::credential_contracts::auth_spec::AuthResolution;
use betterlite_mcp::request_context::McpContext;
use betterlite_mcp::request_headers::raw_headers::RawHeaders;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_server::mcp::debug::auth_diagnostics::{MCPAuthDiagnostics, record_auth_resolution};
use betterlite_server::mcp::debug::headers::{DebugHeaderInputs, DebugServer, build_debug_headers};
use betterlite_server::mcp::debug::maybe_build_debug_headers;
use betterlite_server::mcp::debug::preview::secrets::request_secret_values;
use betterlite_server::mcp::debug::preview::text::safe_upstream_url;
use betterlite_server::mcp::debug::preview::upstream::{
    BodyState, UpstreamFailure, UpstreamRequest, UpstreamResponse, capture_error_response,
};
use betterlite_server::mcp::debug::preview::{
    describe_upstream_http_failure, describe_upstream_response, masked_headers, preview,
    request_body_preview,
};
use betterlite_server::mcp::debug::{DebugSendHeaders, is_debug_enabled, mask_secret};
use futures_util::stream;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use crate::helpers::{HelperError, HelperResult};

use super::input::{
    CaptureInput, CaptureScenario, DebugHeadersInput, DebugScenario, DescribeInput,
    DiagnosticScenario, DiagnosticsInput, FailureInput, Input, MaskInput, Pair, PreviewInput,
    RequestInput, RequestSpec, ResponseSpec, ServerSpec, WrapInput, WrapScenario,
};

pub(crate) fn run(input: &Input) -> HelperResult {
    match input {
        Input::DebugHeaders(value) => debug_headers(value),
        Input::AuthDiagnostics(value) => auth_diagnostics(value),
        Input::WrapSend(value) => wrap_send(value),
        Input::MaskSecret(value) => Ok(masked(value)),
        Input::Preview(value) => previews(value),
        Input::RequestSecrets(value) => request_secrets(value),
        Input::DescribeUpstream(value) => describe(value),
        Input::DescribeFailure(value) => describe_failure(value),
        Input::Capture(value) => capture(value),
    }
}

fn masked(input: &MaskInput) -> Value {
    json!({
        "masks": input.values.iter().map(|value| mask_secret(value.as_deref())).collect::<Vec<_>>(),
    })
}

fn debug_headers(input: &DebugHeadersInput) -> HelperResult {
    let mut results = Vec::with_capacity(input.scenarios.len());
    for scenario in &input.scenarios {
        results.push(debug_scenario(scenario)?);
    }
    Ok(json!({ "results": results }))
}

fn debug_scenario(scenario: &DebugScenario) -> Result<Value, Box<HelperError>> {
    let headers = raw_headers(&scenario.headers)?;
    let oauth2 = scenario
        .oauth2_headers
        .as_ref()
        .map(|pairs| secret_map(pairs));
    let api_key = scenario
        .litellm_api_key
        .as_ref()
        .map(|value| SecretString::from(value.clone()));
    let built = build_debug_headers(&DebugHeaderInputs {
        inbound_headers: &headers,
        oauth2_headers: oauth2.as_ref(),
        litellm_api_key: api_key.as_ref(),
        auth_resolution: auth_resolution(&scenario.auth_resolution)?,
        server_url: scenario.server_url.as_deref(),
        server_auth_type: scenario.server_auth_type.as_deref(),
    });
    let server = find_server(scenario);
    let servers: Vec<DebugServer<'_>> = server.into_iter().collect();
    let maybe = maybe_build_debug_headers(Some(&headers), &servers, oauth2.as_ref());
    Ok(json!({
        "enabled": is_debug_enabled(&headers),
        "headers": built,
        "maybe_headers": maybe,
    }))
}

fn secret_map(pairs: &[Pair]) -> BTreeMap<String, SecretString> {
    let mut map = BTreeMap::new();
    for (name, value) in pairs {
        map.insert(name.clone(), SecretString::from(value.clone()));
    }
    map
}

fn find_server(scenario: &DebugScenario) -> Option<DebugServer<'_>> {
    for name in &scenario.mcp_servers {
        let found = scenario.servers.iter().find(|server: &&ServerSpec| {
            server.name == *name || server.alias.as_deref() == Some(name.as_str())
        });
        if let Some(server) = found {
            return Some(DebugServer {
                url: server.url.as_deref(),
                auth_type: server.auth_type.as_deref(),
            });
        }
    }
    None
}

fn auth_diagnostics(input: &DiagnosticsInput) -> HelperResult {
    let mut results = Vec::with_capacity(input.scenarios.len());
    for scenario in &input.scenarios {
        results.push(diagnostic_scenario(scenario)?);
    }
    Ok(json!({ "results": results }))
}

fn diagnostic_scenario(scenario: &DiagnosticScenario) -> Result<Value, Box<HelperError>> {
    let filled = if scenario.through_context {
        let mut context = McpContext::default();
        context.request_scope.insert(MCPAuthDiagnostics::default());
        for (server_id, resolution) in &scenario.records {
            record_auth_resolution(&mut context, server_id, auth_resolution(resolution)?);
        }
        context
            .request_scope
            .get::<MCPAuthDiagnostics>()
            .cloned()
            .unwrap_or_default()
    } else {
        let mut collected = MCPAuthDiagnostics::default();
        for (server_id, resolution) in &scenario.records {
            collected.record(server_id, auth_resolution(resolution)?);
        }
        collected
    };
    Ok(json!({
        "resolution": filled.resolution().to_string(),
        "headers": filled.headers(),
    }))
}

fn wrap_send(input: &WrapInput) -> HelperResult {
    let mut results = Vec::with_capacity(input.scenarios.len());
    for scenario in &input.scenarios {
        results.push(wrap_scenario(scenario)?);
    }
    Ok(json!({ "results": results }))
}

fn wrap_scenario(scenario: &WrapScenario) -> Result<Value, Box<HelperError>> {
    let mut collected = MCPAuthDiagnostics::default();
    for (server_id, resolution) in &scenario.records {
        collected.record(server_id, auth_resolution(resolution)?);
    }
    let send = DebugSendHeaders {
        debug_headers: scenario.debug_headers.clone(),
        diagnostics: (scenario.method == "POST").then_some(&collected),
    };
    Ok(json!({ "headers": send.headers() }))
}

fn previews(input: &PreviewInput) -> HelperResult {
    let redactor = redactor()?;
    let mut results = Vec::with_capacity(input.scenarios.len());
    for scenario in &input.scenarios {
        results.push(preview(
            scenario.body.as_bytes(),
            scenario.content_type.as_deref().unwrap_or_default(),
            &scenario.secrets,
            &redactor,
        ));
    }
    Ok(json!({ "previews": results }))
}

fn request_secrets(input: &RequestInput) -> HelperResult {
    let redactor = redactor()?;
    let mut results = Vec::with_capacity(input.scenarios.len());
    for spec in &input.scenarios {
        let request = upstream_request(spec)?;
        let secrets = request_secret_values(&request);
        results.push(json!({
            "secrets": secrets,
            "request_preview": request_body_preview(&request, secrets.as_deref(), &redactor),
            "masked_headers": masked_headers(&request.headers),
        }));
    }
    Ok(json!({ "results": results }))
}

fn describe(input: &DescribeInput) -> HelperResult {
    let redactor = redactor()?;
    let mut urls = Vec::with_capacity(input.urls.len());
    for url in &input.urls {
        let parsed = HttpxUrl::parse(url)
            .map_err(|error| Box::new(failure("InvalidURL", error.to_string())))?;
        urls.push(safe_upstream_url(&parsed));
    }
    let mut descriptions = Vec::with_capacity(input.scenarios.len());
    for scenario in &input.scenarios {
        let request = scenario
            .request
            .as_ref()
            .map(upstream_request)
            .transpose()?;
        let response = upstream_response(&scenario.response, request)?;
        descriptions.push(describe_upstream_response(&response, &redactor));
    }
    Ok(json!({ "urls": urls, "descriptions": descriptions }))
}

fn describe_failure(input: &FailureInput) -> HelperResult {
    let redactor = redactor()?;
    let mut responses = Vec::with_capacity(input.responses.len());
    for spec in &input.responses {
        responses.push(upstream_response(spec, None)?);
    }
    let mut descriptions = Vec::with_capacity(input.chains.len());
    for chain in &input.chains {
        let root = failure_chain(chain, &responses);
        descriptions.push(describe_upstream_http_failure(&root, &redactor));
    }
    Ok(json!({ "descriptions": descriptions }))
}

fn capture(input: &CaptureInput) -> HelperResult {
    let redactor = redactor()?;
    let runtime = runtime()?;
    let mut results = Vec::with_capacity(input.scenarios.len());
    for scenario in &input.scenarios {
        results.push(capture_scenario(scenario, &runtime, &redactor)?);
    }
    Ok(json!({ "results": results }))
}

fn capture_scenario(
    scenario: &CaptureScenario,
    runtime: &Runtime,
    redactor: &Redactor,
) -> Result<Value, Box<HelperError>> {
    let request = scenario
        .request
        .as_ref()
        .map(upstream_request)
        .transpose()?;
    let status = StatusCode::from_u16(scenario.status)
        .map_err(|error| Box::new(failure("InvalidStatus", error.to_string())))?;
    let mut response = UpstreamResponse {
        status,
        headers: header_map(&scenario.headers)?,
        body: BodyState::NotRead,
        request,
        captured: None,
    };
    let body = scenario.body.as_bytes().to_vec();
    let chunk_size = scenario.chunk_size.unwrap_or_else(|| body.len().max(1));
    let chunks = chunk_results(&body, chunk_size, scenario.fail_after_chunks);
    runtime.block_on(capture_error_response(
        &mut response,
        stream::iter(chunks),
        redactor,
    ));
    let captured = response.captured.clone();
    let content = match &response.body {
        BodyState::Read(raw) => Some(from_utf8(raw).map(str::to_owned).unwrap_or_default()),
        BodyState::NotRead => None,
    };
    Ok(json!({ "captured": captured, "body": content }))
}

struct StreamBroken;

fn chunk_results(
    body: &[u8],
    chunk_size: usize,
    fail_after: Option<usize>,
) -> Vec<Result<Vec<u8>, StreamBroken>> {
    let mut results = Vec::new();
    let mut offset = 0_usize;
    while offset < body.len() {
        if fail_after.is_some_and(|limit| results.len() >= limit) {
            results.push(Err(StreamBroken));
            return results;
        }
        let end = offset.saturating_add(chunk_size).min(body.len());
        results.push(Ok(body.get(offset..end).unwrap_or_default().to_vec()));
        offset = end;
    }
    results
}

struct FailureNode {
    response: Option<UpstreamResponse>,
    cause: Option<Box<Self>>,
}

impl UpstreamFailure for FailureNode {
    fn response(&self) -> Option<&UpstreamResponse> {
        self.response.as_ref()
    }

    fn cause(&self) -> Option<&dyn UpstreamFailure> {
        let node = self.cause.as_deref()?;
        Some(node)
    }
}

fn failure_chain(entries: &[Option<usize>], responses: &[UpstreamResponse]) -> FailureNode {
    let mut cause: Option<Box<FailureNode>> = None;
    for entry in entries.iter().rev() {
        let response = entry.and_then(|index| responses.get(index)).cloned();
        cause = Some(Box::new(FailureNode { response, cause }));
    }
    cause.map_or_else(
        || FailureNode {
            response: None,
            cause: None,
        },
        |node| *node,
    )
}

fn upstream_request(spec: &RequestSpec) -> Result<UpstreamRequest, Box<HelperError>> {
    let body = spec.body.as_ref().map_or(BodyState::NotRead, |body| {
        BodyState::Read(body.as_bytes().to_vec())
    });
    let parsed = HttpxUrl::parse(&spec.url)
        .map_err(|error| Box::new(failure("InvalidURL", error.to_string())))?;
    Ok(UpstreamRequest {
        method: spec.method.clone(),
        url: parsed,
        headers: header_map(&spec.headers)?,
        body,
    })
}

fn upstream_response(
    spec: &ResponseSpec,
    request: Option<UpstreamRequest>,
) -> Result<UpstreamResponse, Box<HelperError>> {
    let body = spec.body.as_ref().map_or(BodyState::NotRead, |body| {
        BodyState::Read(body.as_bytes().to_vec())
    });
    let status = StatusCode::from_u16(spec.status)
        .map_err(|error| Box::new(failure("InvalidStatus", error.to_string())))?;
    Ok(UpstreamResponse {
        status,
        headers: header_map(&spec.headers)?,
        body,
        request,
        captured: spec.captured.clone(),
    })
}
fn header_map(pairs: &[Pair]) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| Box::new(failure("InvalidHeader", error.to_string())))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|error| Box::new(failure("InvalidHeader", error.to_string())))?;
        headers.append(header_name, header_value);
    }
    Ok(headers)
}

fn raw_headers(pairs: &[Pair]) -> Result<RawHeaders, Box<HelperError>> {
    let bytes: Vec<(&[u8], &[u8])> = pairs
        .iter()
        .map(|(name, value)| (name.as_bytes(), value.as_bytes()))
        .collect();
    RawHeaders::from_pairs(&bytes).map_err(|_duplicate| {
        let mut error = failure(
            "HTTPException",
            "Multiple Authorization headers are not allowed".to_owned(),
        );
        error.status = Some(400);
        Box::new(error)
    })
}
fn redactor() -> Result<Redactor, Box<HelperError>> {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| Box::new(failure("RedactionError", error.to_string())))
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| Box::new(failure("RuntimeError", error.to_string())))
}

fn auth_resolution(name: &str) -> Result<AuthResolution, Box<HelperError>> {
    match name {
        "no-auth" => Ok(AuthResolution::NoAuth),
        "stored-user-token" => Ok(AuthResolution::StoredUserToken),
        "static-token" => Ok(AuthResolution::StaticToken),
        "per-request-header" => Ok(AuthResolution::PerRequestHeader),
        "oauth2-passthrough" => Ok(AuthResolution::Oauth2Passthrough),
        "m2m-client-credentials" => Ok(AuthResolution::ClientCredentials),
        "token-exchange" => Ok(AuthResolution::TokenExchange),
        "id-jag" => Ok(AuthResolution::IdJag),
        "aws-sigv4" => Ok(AuthResolution::AwsSigv4),
        "extra-headers" => Ok(AuthResolution::ExtraHeaders),
        "not-applicable" => Ok(AuthResolution::NotApplicable),
        "unresolved" => Ok(AuthResolution::Unresolved),
        "resolution-failed" => Ok(AuthResolution::Failed),
        "multiple" => Ok(AuthResolution::Multiple),
        other => Err(Box::new(failure(
            "ValueError",
            format!("unknown auth resolution: {other}"),
        ))),
    }
}

fn failure(class: &str, message: String) -> HelperError {
    HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    }
}
