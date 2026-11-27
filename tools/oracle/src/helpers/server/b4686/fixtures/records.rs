use std::collections::BTreeMap;

use betterlite_mcp::request_headers::raw_headers::RawHeaders;
use betterlite_server::management_rbac::FeatureName;
use betterlite_server::mcp::hook_payloads::GuardrailContext;
use betterlite_server::mcp::search_handlers::ports::SearchPortError;
use betterlite_server::mcp::search_handlers::types::{CallerScope, McpRequestScope};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value as JsonValue;
use serde_json::json;

#[derive(Default)]
pub(super) struct Records {
    pub(super) feature_checks: Vec<JsonValue>,
    pub(super) agent_searches: Vec<JsonValue>,
    pub(super) skill_searches: Vec<JsonValue>,
    pub(super) list_calls: Vec<JsonValue>,
    pub(super) allowed_calls: Vec<JsonValue>,
    pub(super) denials: Vec<JsonValue>,
    pub(super) executions: Vec<JsonValue>,
    pub(super) embeddings: Vec<JsonValue>,
}

impl Records {
    pub(super) fn value(&self) -> JsonValue {
        json!({
            "feature_checks": self.feature_checks,
            "agent_searches": self.agent_searches,
            "skill_searches": self.skill_searches,
            "list_calls": self.list_calls,
            "allowed_calls": self.allowed_calls,
            "denials": self.denials,
            "executions": self.executions,
            "embeddings": self.embeddings,
        })
    }
}

pub(super) fn caller_json(caller: &CallerScope) -> JsonValue {
    json!({"user_id": caller.user_id, "end_user_id": caller.end_user_id})
}

pub(super) const fn feature_name(feature: FeatureName) -> &'static str {
    match feature {
        FeatureName::Agents => "agents",
        FeatureName::VectorStores => "vector_stores",
    }
}

pub(super) fn scope_json(scope: &McpRequestScope) -> JsonValue {
    json!({
        "client_ip": scope.client_ip,
        "mcp_servers": scope.mcp_servers,
        "mcp_auth_header": secret_value(scope.mcp_auth_header.as_ref()),
        "mcp_server_auth_headers": secret_headers_json(&scope.mcp_server_auth_headers),
        "oauth2_headers": secret_map_json(&scope.oauth2_headers),
        "raw_headers": raw_headers_json(&scope.raw_headers),
    })
}

pub(super) fn execution_scope_json(scope: &McpRequestScope) -> JsonValue {
    let mut execution = scope_json(scope);
    if let JsonValue::Object(fields) = &mut execution {
        fields.remove("mcp_servers");
    }
    execution
}

pub(super) fn allowed_call_json(caller: &CallerScope, scope: &McpRequestScope) -> JsonValue {
    json!({
        "caller": caller_json(caller),
        "scope": {"client_ip": scope.client_ip, "mcp_servers": scope.mcp_servers},
    })
}

pub(super) fn guardrail_json(
    context: Option<&GuardrailContext>,
) -> Result<JsonValue, SearchPortError> {
    let Some(context) = context else {
        return Ok(JsonValue::Null);
    };
    let metadata = serde_json::to_value(&context.metadata).map_err(|error| SearchPortError {
        detail: error.to_string(),
    })?;
    Ok(json!({"model": context.model, "metadata": metadata}))
}

fn secret_value(value: Option<&SecretString>) -> Option<&str> {
    value.map(|secret| secret.expose_secret())
}

fn secret_headers_json(headers: &BTreeMap<String, BTreeMap<String, SecretString>>) -> JsonValue {
    let exposed: BTreeMap<&str, BTreeMap<&str, &str>> = headers
        .iter()
        .map(|(server, fields)| {
            let fields: BTreeMap<&str, &str> = fields
                .iter()
                .map(|(name, value)| (name.as_str(), value.expose_secret()))
                .collect();
            (server.as_str(), fields)
        })
        .collect();
    json!(exposed)
}

fn secret_map_json(headers: &BTreeMap<String, SecretString>) -> JsonValue {
    let exposed: BTreeMap<&str, &str> = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect();
    json!(exposed)
}

fn raw_headers_json(headers: &RawHeaders) -> JsonValue {
    let exposed: BTreeMap<&str, &str> = headers.items().into_iter().collect();
    json!(exposed)
}
