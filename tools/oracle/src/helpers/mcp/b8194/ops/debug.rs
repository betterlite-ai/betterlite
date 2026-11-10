use betterlite_mcp::request_context::McpContext;
use betterlite_mcp::request_headers::raw_headers::RawHeaders;
use betterlite_server::mcp::debug::auth_diagnostics::{MCPAuthDiagnostics, record_auth_resolution};
use betterlite_server::mcp::debug::headers::{DebugHeaderInputs, DebugServer, build_debug_headers};
use betterlite_server::mcp::debug::maybe_build_debug_headers;
use betterlite_server::mcp::debug::{DebugSendHeaders, is_debug_enabled};
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

use super::super::input::{
    DebugHeadersInput, DebugScenario, DiagnosticScenario, DiagnosticsInput, Pair, ServerSpec,
    WrapInput, WrapScenario,
};
use super::{auth_resolution, failure, secret_map};

pub(super) fn debug_headers(input: &DebugHeadersInput) -> HelperResult {
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

pub(super) fn auth_diagnostics(input: &DiagnosticsInput) -> HelperResult {
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

pub(super) fn wrap_send(input: &WrapInput) -> HelperResult {
    let mut results: Vec<Value> = Vec::with_capacity(input.scenarios.len());
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
