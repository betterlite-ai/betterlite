use std::collections::BTreeMap;

use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_mcp::credential_contracts::auth_spec::AuthResolution;
use betterlite_mcp::request_context::McpContext;
use betterlite_mcp::request_headers::raw_headers::RawHeaders;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_server::mcp::debug::auth_diagnostics::{MCPAuthDiagnostics, record_auth_resolution};
use betterlite_server::mcp::debug::headers::{DebugHeaderInputs, build_debug_headers};
use betterlite_server::mcp::debug::is_debug_enabled;
use betterlite_server::mcp::debug::preview::preview;
use betterlite_server::mcp::debug::preview::text::safe_upstream_url;
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

use super::input::{BodyScenario, DebugScenario, DiagnosticScenario, Input, Operation, Pair};

pub(crate) fn run(input: &Input) -> HelperResult {
    let Input {
        operation: Operation::Run,
        ..
    } = input;
    let redactor = redactor()?;
    let mut debug: Vec<Value> = Vec::with_capacity(input.debug.len());
    for scenario in &input.debug {
        debug.push(debug_scenario(scenario)?);
    }
    let mut diagnostics: Vec<Value> = Vec::with_capacity(input.diagnostics.len());
    for scenario in &input.diagnostics {
        diagnostics.push(diagnostic_scenario(scenario)?);
    }
    let mut bodies: Vec<Value> = Vec::with_capacity(input.bodies.len());
    for scenario in &input.bodies {
        bodies.push(body_scenario(scenario, &redactor)?);
    }
    Ok(json!({
        "debug": debug,
        "diagnostics": diagnostics,
        "bodies": bodies,
    }))
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
    Ok(json!({
        "enabled": is_debug_enabled(&headers),
        "headers": built,
    }))
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

fn body_scenario(scenario: &BodyScenario, redactor: &Redactor) -> Result<Value, Box<HelperError>> {
    let url = safe_url(scenario.url.as_ref())?;
    let body = scenario.body.as_bytes();
    let content_type = scenario.content_type.as_deref().unwrap_or_default();
    let secrets: Vec<SecretString> = scenario
        .secrets
        .iter()
        .map(|secret| SecretString::from(secret.clone()))
        .collect();
    Ok(json!({ "preview": preview(body, content_type, &secrets, redactor), "url": url }))
}

fn safe_url(source: Option<&String>) -> Result<Option<String>, Box<HelperError>> {
    let Some(address) = source else {
        return Ok(None);
    };
    let parsed = HttpxUrl::parse(address).map_err(|error| {
        Box::new(HelperError {
            class: "InvalidURL".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    Ok(Some(safe_upstream_url(&parsed)))
}

fn secret_map(pairs: &[Pair]) -> BTreeMap<String, SecretString> {
    let mut map = BTreeMap::new();
    for (name, value) in pairs {
        map.insert(name.clone(), SecretString::from(value.clone()));
    }
    map
}

fn raw_headers(pairs: &[Pair]) -> Result<RawHeaders, Box<HelperError>> {
    let bytes: Vec<(&[u8], &[u8])> = pairs
        .iter()
        .map(|(name, value)| (name.as_bytes(), value.as_bytes()))
        .collect();
    RawHeaders::from_pairs(&bytes).map_err(|_duplicate| {
        Box::new(HelperError {
            class: "HTTPException".to_owned(),
            status: Some(400),
            error_type: None,
            param: None,
            code: None,
            message: "Multiple Authorization headers are not allowed".to_owned(),
        })
    })
}

fn redactor() -> Result<Redactor, Box<HelperError>> {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| {
        Box::new(HelperError {
            class: "RedactionError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
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
        other => Err(Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: format!("unknown auth resolution: {other}"),
        })),
    }
}
