use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_mcp::request_headers::client_headers::{
    get_litellm_api_key, mcp_auth_header, oauth2_headers, parse_mcp_access_groups_header,
    parse_mcp_servers_header, server_auth_headers,
};
use betterlite_mcp::request_headers::raw_headers::RawHeaders;
use serde_json::{Value, json};

use super::support;
use crate::helpers::{HelperError, HelperResult};

const ENV_NAME: &str = "LITELLM_MCP_CLIENT_SIDE_AUTH_HEADER_NAME";

pub(super) fn dispatch(function: &str, input: &Value) -> HelperResult {
    let headers = support::parse_headers(&support::pairs(input)?)?;
    match function {
        "safe_headers" => {
            let mut lookups: BTreeMap<String, Option<&str>> = BTreeMap::new();
            for name in support::strings(input, "lookups") {
                lookups.insert(name.to_owned(), headers.lookup(name));
            }
            Ok(json!({"items": headers.items(), "lookups": lookups}))
        }
        "litellm_key" => {
            let key = get_litellm_api_key(&headers);
            Ok(json!({
                "value": key.value,
                "explicit_header_present": key.explicit_header_present,
            }))
        }
        "mcp_auth_header" => {
            let header_name = config_header_name(input)?;
            let value = mcp_auth_header(&headers, &header_name);
            Ok(json!({"header_name": header_name, "value": value}))
        }
        _ => lists(function, input, &headers),
    }
}

fn lists(function: &str, input: &Value, headers: &RawHeaders) -> HelperResult {
    match function {
        "server_auth" => Ok(json!({"value": server_auth_headers(headers)})),
        "oauth2" => Ok(json!({"value": oauth2_headers(headers)})),
        "list_header" => {
            if support::optional(input, "header") == Some("access_groups") {
                return Ok(json!({"value": parse_mcp_access_groups_header(headers)}));
            }
            Ok(json!({"value": parse_mcp_servers_header(headers)}))
        }
        other => Err(support::invalid_operation(other)),
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
