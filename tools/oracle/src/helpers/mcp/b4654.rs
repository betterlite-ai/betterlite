mod support;

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_mcp::credential_contracts::token_endpoint_auth::TokenEndpointClientAuth;
use betterlite_mcp::oauth_resource::{base_url, challenge, resource_identity};
use betterlite_mcp::request_headers::{client_headers, path_targets};
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

const ENV_NAME: &str = "LITELLM_MCP_CLIENT_SIDE_AUTH_HEADER_NAME";

impl Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> HelperResult {
        let function = support::function_name(input)?;
        match function {
            "strip_default_port" => Ok(
                json!({"netloc": base_url::strip_default_port(support::string(input, "scheme")?, support::string(input, "netloc")?)}),
            ),
            "proxy_base_url" => Ok(
                json!({"value": base_url::normalize_proxy_base_url(support::string(input, "raw")?)}),
            ),
            "request_base_url" => Ok(json!({"base_url": request_base(input)?})),
            "well_known_root_suffix" => Ok(
                json!({"value": challenge::well_known_root_suffix(support::optional(input, "server_root_path").unwrap_or_default())}),
            ),
            "route_relative_path" => Ok(
                json!({"value": challenge::route_relative_request_path(support::optional(input, "original_path"), support::optional(input, "path").unwrap_or_default(), support::optional(input, "app_root_path"), support::optional(input, "root_path"))}),
            ),
            "metadata_url" => Ok(json!({"value": metadata(input)})),
            "www_authenticate" => Ok(
                json!({"value": challenge::passthrough_www_authenticate(&metadata(input), support::boolean(input, "invalid_token")?)}),
            ),
            "byok_authenticate" => Ok(
                json!({"value": challenge::byok_www_authenticate(support::optional(input, "proxy_base_url").unwrap_or_default(), support::optional(input, "request_root_path").unwrap_or_default())}),
            ),
            "canonicalize" => Ok(
                json!({"value": if support::boolean(input, "resource_only").unwrap_or(false) { json!(resource_identity::canonical_resource_uri(support::string(input, "url")?)?) } else { json!(resource_identity::canonicalize_url_identity(support::string(input, "url")?)?) }}),
            ),
            "redact_url" => Ok(
                json!({"value": resource_identity::redact_mcp_resource_url(support::optional(input, "url"))}),
            ),
            "resolve_resource" => {
                Ok(json!({"value": resource_identity::resolve_upstream_resource(&upstream(input))}))
            }
            "token_request" => Ok(json!(token_body(input)?)),
            "safe_headers" => Ok(json!(safe_header_values(input)?)),
            "litellm_key" => Ok(json!(litellm_key_value(input)?)),
            "mcp_auth_header" => Ok(json!(mcp_auth_value(input)?)),
            "server_auth" => Ok(
                json!({"value": client_headers::server_auth_headers(&support::parse_headers(&support::pairs(input)?)?)}),
            ),
            "oauth2" => Ok(
                json!({"value": client_headers::oauth2_headers(&support::parse_headers(&support::pairs(input)?)?)}),
            ),
            "list_header" => Ok(json!(list_header_value(input)?)),
            "target_names" => Ok(json!(target_values(input))),
            "target_selection" => Ok(json!(target_selection(input)?)),
            other => Err(support::invalid_operation(other)),
        }
    }
}

fn request_base(input: &Value) -> Result<String, Box<HelperError>> {
    let origin = base_url::RequestOrigin {
        base_url: support::string(input, "base_url")?,
        x_forwarded_proto: support::optional(input, "x_forwarded_proto"),
        x_forwarded_host: support::optional(input, "x_forwarded_host"),
        x_forwarded_port: support::optional(input, "x_forwarded_port"),
        from_trusted_proxy: support::boolean(input, "from_trusted_proxy")?,
    };
    let value = base_url::get_request_base_url(
        support::optional(input, "proxy_base_url").unwrap_or_default(),
        &origin,
    )?;
    Ok(value)
}

fn metadata(input: &Value) -> String {
    let base = support::optional(input, "base_url").unwrap_or_default();
    let route = support::optional(input, "route_path").unwrap_or_default();
    let name = support::optional(input, "server_name").unwrap_or_default();
    let root = challenge::well_known_root_suffix(
        support::optional(input, "server_root_path").unwrap_or_default(),
    );
    challenge::passthrough_metadata_url(base, route, name, root)
}

fn upstream(input: &Value) -> resource_identity::UpstreamServer<'_> {
    resource_identity::UpstreamServer {
        upstream_resource: support::optional(input, "upstream_resource"),
        url: support::optional(input, "url"),
        server_id: support::optional(input, "server_id").unwrap_or_default(),
    }
}

fn token_client_auth(input: &Value) -> Result<TokenEndpointClientAuth, Box<HelperError>> {
    let auth_method = support::optional(input, "auth_method");
    let client_id = support::optional(input, "client_id");
    let secret =
        support::optional(input, "client_secret").map(|value| SecretString::from(value.to_owned()));
    Ok(resource_identity::build_upstream_token_request(
        &upstream(input),
        auth_method,
        client_id,
        secret.as_ref(),
    )?)
}

fn token_body(input: &Value) -> Result<Value, Box<HelperError>> {
    let auth = token_client_auth(input)?;
    let headers = support::expose(auth.headers);
    let body = support::expose(auth.body);
    Ok(json!({"headers": headers, "body": body}))
}

fn safe_header_values(input: &Value) -> Result<Value, Box<HelperError>> {
    let headers = support::parse_headers(&support::pairs(input)?)?;
    let mut lookups: BTreeMap<String, Option<&str>> = BTreeMap::new();
    for name in support::strings(input, "lookups").unwrap_or_default() {
        lookups.insert(name.to_owned(), headers.lookup(name));
    }
    Ok(json!({"items": headers.items(), "lookups": lookups}))
}

fn litellm_key_value(input: &Value) -> Result<Value, Box<HelperError>> {
    let headers = support::parse_headers(&support::pairs(input)?)?;
    let key = client_headers::get_litellm_api_key(&headers);
    Ok(json!({"value": key.value, "explicit_header_present": key.explicit_header_present}))
}

fn mcp_auth_value(input: &Value) -> Result<Value, Box<HelperError>> {
    let header_name = config_header_name(input)?;
    let headers = support::parse_headers(&support::pairs(input)?)?;
    let value = client_headers::mcp_auth_header(&headers, &header_name);
    Ok(json!({"header_name": header_name, "value": value}))
}

fn list_header_value(input: &Value) -> Result<Value, Box<HelperError>> {
    let headers = support::parse_headers(&support::pairs(input)?)?;
    if support::optional(input, "header") == Some("access_groups") {
        return Ok(json!({"value": client_headers::parse_mcp_access_groups_header(&headers)}));
    }
    Ok(json!({"value": client_headers::parse_mcp_servers_header(&headers)}))
}

fn target_values(input: &Value) -> Value {
    let paths = support::strings(input, "paths").unwrap_or_default();
    let values: Vec<Vec<String>> = paths
        .iter()
        .map(|path| path_targets::target_names_from_path(path))
        .collect();
    json!({"values": values})
}

fn target_selection(input: &Value) -> Result<Value, Box<HelperError>> {
    let servers = header_servers(input);
    let path = support::string(input, "path")?;
    if support::optional(input, "mode") == Some("cold_start") {
        return Ok(
            json!({"value": path_targets::cold_start_server_names(path, servers.as_deref()).map(|name| vec![name])}),
        );
    }
    Ok(json!({"value": path_targets::resolve_target_server_names(path, servers.as_deref())}))
}

fn header_servers(input: &Value) -> Option<Vec<String>> {
    let names = support::strings(input, "header_servers")?;
    let mut servers = Vec::with_capacity(names.len());
    for name in names {
        servers.push(name.to_owned());
    }
    Some(servers)
}

fn config_header_name(input: &Value) -> Result<String, Box<HelperError>> {
    let mut lookup = BTreeMap::new();
    if let Some(value) = support::optional(input, "env_header_name") {
        lookup.insert(ENV_NAME.to_owned(), value.to_owned());
    }
    let document = support::optional(input, "general_settings").map_or_else(
        || "{}".to_owned(),
        |value| {
            json!({"general_settings": {"mcp_client_side_auth_header_name": value}}).to_string()
        },
    );
    let config = ConfigLoader { lookup }
        .load(&document)
        .map_err(|error| support::invalid("ValueError", None, error.to_string()))?;
    Ok(config.sections.mcp_client_side_auth.header_name)
}
