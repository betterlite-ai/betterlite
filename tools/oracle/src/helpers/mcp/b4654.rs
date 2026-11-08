mod support;

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_mcp::credential_contracts::token_endpoint_auth::TokenEndpointClientAuth;
use betterlite_mcp::oauth_resource::base_url::{
    RequestOrigin, get_request_base_url, normalize_proxy_base_url, strip_default_port,
};
use betterlite_mcp::oauth_resource::challenge::{
    byok_www_authenticate, passthrough_metadata_url, passthrough_www_authenticate,
    route_relative_request_path, well_known_root_suffix,
};
use betterlite_mcp::oauth_resource::resource_identity::{
    UpstreamServer, build_upstream_token_request, canonical_resource_uri,
    canonicalize_url_identity, redact_mcp_resource_url, resolve_upstream_resource,
};
use betterlite_mcp::request_headers::client_headers::{
    get_litellm_api_key, mcp_auth_header, oauth2_headers, parse_mcp_access_groups_header,
    parse_mcp_servers_header, server_auth_headers,
};
use betterlite_mcp::request_headers::path_targets::{
    cold_start_server_names, resolve_target_server_names, target_names_from_path,
};
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

const ENV_NAME: &str = "LITELLM_MCP_CLIENT_SIDE_AUTH_HEADER_NAME";

impl Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> HelperResult {
        let function = input
            .get("function")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                support::invalid(
                    "InvalidInput",
                    None,
                    "every b4654 call names a function".to_owned(),
                )
            })?;
        match function {
            "strip_default_port" | "proxy_base_url" | "request_base_url" => base(function, input),
            "well_known_root_suffix"
            | "route_relative_path"
            | "metadata_url"
            | "www_authenticate"
            | "byok_authenticate" => challenges(function, input),
            "canonicalize" | "redact_url" | "resolve_resource" | "token_request" => {
                resource(function, input)
            }
            "safe_headers" | "litellm_key" | "mcp_auth_header" | "server_auth" | "oauth2"
            | "list_header" => headers(function, input),
            "target_names" | "target_selection" => paths(function, input),
            other => Err(support::invalid_operation(other)),
        }
    }
}

fn base(function: &str, input: &Value) -> HelperResult {
    match function {
        "strip_default_port" => Ok(
            json!({"netloc": strip_default_port(support::string(input, "scheme")?, support::string(input, "netloc")?)}),
        ),
        "proxy_base_url" => {
            Ok(json!({"value": normalize_proxy_base_url(support::string(input, "raw")?)}))
        }
        _ => Ok(
            json!({"base_url": get_request_base_url(support::optional(input, "proxy_base_url").unwrap_or_default(), &RequestOrigin { base_url: support::string(input, "base_url")?, x_forwarded_proto: support::optional(input, "x_forwarded_proto"), x_forwarded_host: support::optional(input, "x_forwarded_host"), x_forwarded_port: support::optional(input, "x_forwarded_port"), from_trusted_proxy: support::boolean(input, "from_trusted_proxy")? })?}),
        ),
    }
}

fn challenges(function: &str, input: &Value) -> HelperResult {
    match function {
        "well_known_root_suffix" => Ok(
            json!({"value": well_known_root_suffix(support::optional(input, "server_root_path").unwrap_or_default())}),
        ),
        "route_relative_path" => Ok(
            json!({"value": route_relative_request_path(support::optional(input, "original_path"), support::optional(input, "path").unwrap_or_default(), support::optional(input, "app_root_path"), support::optional(input, "root_path"))}),
        ),
        "byok_authenticate" => Ok(
            json!({"value": byok_www_authenticate(support::optional(input, "proxy_base_url").unwrap_or_default(), support::optional(input, "request_root_path").unwrap_or_default())}),
        ),
        "www_authenticate" => Ok(
            json!({"value": passthrough_www_authenticate(&metadata(input), support::boolean(input, "invalid_token")?)}),
        ),
        _ => Ok(json!({"value": metadata(input)})),
    }
}

fn metadata(input: &Value) -> String {
    let base_url = support::optional(input, "base_url").unwrap_or_default();
    let route_path = support::optional(input, "route_path").unwrap_or_default();
    let server_name = support::optional(input, "server_name").unwrap_or_default();
    let root_suffix =
        well_known_root_suffix(support::optional(input, "server_root_path").unwrap_or_default());
    passthrough_metadata_url(base_url, route_path, server_name, root_suffix)
}

fn resource(function: &str, input: &Value) -> HelperResult {
    match function {
        "canonicalize" => Ok(
            json!({"value": if support::boolean(input, "resource_only").unwrap_or(false) { json!(canonical_resource_uri(support::string(input, "url")?)?) } else { json!(canonicalize_url_identity(support::string(input, "url")?)?) }}),
        ),
        "redact_url" => {
            Ok(json!({"value": redact_mcp_resource_url(support::optional(input, "url"))}))
        }
        "resolve_resource" => Ok(json!({"value": resolve_upstream_resource(&upstream(input))})),
        _ => {
            let auth = token_client_auth(input)?;
            Ok(
                json!({"headers": support::expose(auth.headers), "body": support::expose(auth.body)}),
            )
        }
    }
}

fn upstream(input: &Value) -> UpstreamServer<'_> {
    UpstreamServer {
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
    Ok(build_upstream_token_request(
        &upstream(input),
        auth_method,
        client_id,
        secret.as_ref(),
    )?)
}

fn headers(function: &str, input: &Value) -> HelperResult {
    let parsed = support::parse_headers(&support::pairs(input)?)?;
    match function {
        "safe_headers" => {
            let mut lookups: BTreeMap<String, Option<&str>> = BTreeMap::new();
            for name in support::strings(input, "lookups").unwrap_or_default() {
                lookups.insert(name.to_owned(), parsed.lookup(name));
            }
            Ok(json!({"items": parsed.items(), "lookups": lookups}))
        }
        "litellm_key" => {
            let key = get_litellm_api_key(&parsed);
            Ok(json!({"value": key.value, "explicit_header_present": key.explicit_header_present}))
        }
        "mcp_auth_header" => {
            let header_name = config_header_name(input)?;
            Ok(json!({"header_name": header_name, "value": mcp_auth_header(&parsed, &header_name)}))
        }
        "server_auth" => Ok(json!({"value": server_auth_headers(&parsed)})),
        "oauth2" => Ok(json!({"value": oauth2_headers(&parsed)})),
        _ => Ok(
            json!({"value": if support::optional(input, "header") == Some("access_groups") { parse_mcp_access_groups_header(&parsed) } else { parse_mcp_servers_header(&parsed) }}),
        ),
    }
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

fn paths(function: &str, input: &Value) -> HelperResult {
    if function == "target_names" {
        let values: Vec<Vec<String>> = support::strings(input, "paths")
            .unwrap_or_default()
            .iter()
            .map(|path| target_names_from_path(path))
            .collect();
        return Ok(json!({"values": values}));
    }
    let servers = header_servers(input);
    let path = support::string(input, "path")?;
    if support::optional(input, "mode") == Some("cold_start") {
        return Ok(
            json!({"value": cold_start_server_names(path, servers.as_deref()).map(|name| vec![name])}),
        );
    }
    Ok(json!({"value": resolve_target_server_names(path, servers.as_deref())}))
}

fn header_servers(input: &Value) -> Option<Vec<String>> {
    let names = support::strings(input, "header_servers")?;
    let mut servers = Vec::with_capacity(names.len());
    for name in names {
        servers.push(name.to_owned());
    }
    Some(servers)
}
