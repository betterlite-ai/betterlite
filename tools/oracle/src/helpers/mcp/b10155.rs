mod input;
mod tests;

use std::collections::BTreeMap;

use betterlite_mcp::client_auth_headers::client_has_passthrough_authorization; // ast-grep-ignore: no-long-names - the owned predicate keeps LiteLLM's name
use betterlite_mcp::client_auth_headers::client_has_per_server_auth_header; // ast-grep-ignore: no-long-names - the owned predicate keeps LiteLLM's name
use betterlite_mcp::client_auth_headers::lookup::ServerAuthHeaders;
use betterlite_mcp::client_auth_headers::lookup::lookup_server_auth;
use betterlite_mcp::client_auth_headers::lookup::sanitize_alias_for_header;
use betterlite_mcp::server_record::MCPServer;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;

use input::{Call, HeaderMap, Input, ServerSpec};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answers = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            answers.push(answer(call)?);
        }
        Ok(json!({ "answers": answers }))
    }
}

fn answer(call: &Call) -> HelperResult {
    match call {
        Call::SanitizeAlias { alias } => Ok(json!({
            "sanitized": sanitize_alias_for_header(alias),
        })),
        Call::Lookup {
            headers,
            alias,
            server_name,
            access_groups,
        } => lookup_answer(
            headers,
            alias.as_deref(),
            server_name.as_deref(),
            access_groups,
        ),
        Call::PerServerAuthorization {
            server,
            server_auth_headers,
        } => per_server_answer(server, server_auth_headers.as_ref()),
        Call::PassthroughAuthorization {
            server,
            server_auth_headers,
            oauth2_headers,
        } => passthrough_answer(
            server,
            server_auth_headers.as_ref(),
            oauth2_headers.as_ref(),
        ),
    }
}

fn lookup_answer(
    headers: &HeaderMap,
    alias: Option<&str>,
    server_name: Option<&str>,
    access_groups: &[String],
) -> HelperResult {
    let built = header_map(headers);
    let resolved = lookup_server_auth(&built, alias, server_name, access_groups);
    Ok(json!({ "resolved": resolved.map(exposed) }))
}

fn per_server_answer(server: &ServerSpec, server_auth_headers: Option<&HeaderMap>) -> HelperResult {
    let built = build_server(server)?;
    let per_server = server_auth_headers.map(header_map);
    // ast-grep-ignore: no-long-names - the owned predicate keeps LiteLLM's name
    let has_header = client_has_per_server_auth_header(&built, per_server.as_ref());
    Ok(json!({ "has_header": has_header }))
}

fn passthrough_answer(
    server: &ServerSpec,
    server_auth_headers: Option<&HeaderMap>,
    oauth2_headers: Option<&BTreeMap<String, String>>,
) -> HelperResult {
    let built = build_server(server)?;
    let per_server = server_auth_headers.map(header_map);
    let top_level = oauth2_headers.map(top_level_map);
    // ast-grep-ignore: no-long-names - the owned predicate keeps LiteLLM's name
    let has_passthrough =
        client_has_passthrough_authorization(&built, top_level.as_ref(), per_server.as_ref());
    Ok(json!({ "has_passthrough": has_passthrough }))
}

fn header_map(source: &HeaderMap) -> ServerAuthHeaders {
    let mut converted = ServerAuthHeaders::new();
    for (alias, headers) in source {
        let mut inner = BTreeMap::new();
        for (name, value) in headers {
            inner.insert(name.clone(), SecretString::from(value.clone()));
        }
        converted.insert(alias.clone(), inner);
    }
    converted
}

fn top_level_map(source: &BTreeMap<String, String>) -> BTreeMap<String, SecretString> {
    let mut converted = BTreeMap::new();
    for (name, value) in source {
        converted.insert(name.clone(), SecretString::from(value.clone()));
    }
    converted
}

fn exposed(headers: &BTreeMap<String, SecretString>) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for (name, value) in headers {
        values.insert(name.clone(), value.expose_secret().to_owned());
    }
    values
}

fn build_server(spec: &ServerSpec) -> Result<MCPServer, Box<HelperError>> {
    let record = json!({
        "server_id": "test-server",
        "name": "test-server",
        "transport": "http",
        "alias": spec.alias,
        "server_name": spec.server_name,
        "access_groups": spec.access_groups,
    });
    serde_json::from_value(record).or_value_error()
}
