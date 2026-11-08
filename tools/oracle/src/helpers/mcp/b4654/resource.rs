use betterlite_mcp::credential_contracts::token_endpoint_auth::TokenEndpointClientAuth;
use betterlite_mcp::oauth_resource::resource_identity::{
    UpstreamServer, build_upstream_token_request, canonical_resource_uri,
    canonicalize_url_identity, redact_mcp_resource_url, resolve_upstream_resource,
};
use secrecy::SecretString;
use serde_json::{Value, json};

use super::support;
use crate::helpers::{HelperError, HelperResult};

pub(super) fn dispatch(function: &str, input: &Value) -> HelperResult {
    match function {
        "canonicalize" => {
            let url = support::string(input, "url")?;
            if support::boolean(input, "resource_only").unwrap_or(false) {
                return Ok(json!({"value": canonical_resource_uri(url)?}));
            }
            Ok(json!({"value": canonicalize_url_identity(url)?}))
        }
        "redact_url" => Ok(json!({
            "value": redact_mcp_resource_url(support::optional(input, "url"))
        })),
        _ => upstream(function, input),
    }
}

fn upstream(function: &str, input: &Value) -> HelperResult {
    match function {
        "resolve_resource" => Ok(json!({"value": resolve_upstream_resource(&server(input))})),
        "token_request" => {
            let auth = token_request(input)?;
            Ok(
                json!({"headers": support::expose(auth.headers), "body": support::expose(auth.body)}),
            )
        }
        other => Err(support::invalid_operation(other)),
    }
}

fn server(input: &Value) -> UpstreamServer<'_> {
    UpstreamServer {
        upstream_resource: support::optional(input, "upstream_resource"),
        url: support::optional(input, "url"),
        server_id: support::optional(input, "server_id").unwrap_or_default(),
    }
}

fn token_request(input: &Value) -> Result<TokenEndpointClientAuth, Box<HelperError>> {
    let auth_method = support::optional(input, "auth_method");
    let client_id = support::optional(input, "client_id");
    let secret =
        support::optional(input, "client_secret").map(|value| SecretString::from(value.to_owned()));
    Ok(build_upstream_token_request(
        &server(input),
        auth_method,
        client_id,
        secret.as_ref(),
    )?)
}
