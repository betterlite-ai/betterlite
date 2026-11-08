use betterlite_mcp::oauth_resource::challenge::{
    byok_www_authenticate, passthrough_metadata_url, passthrough_www_authenticate,
    route_relative_request_path, well_known_root_suffix,
};
use serde_json::{Value, json};

use super::support;
use crate::helpers::HelperResult;

pub(super) fn dispatch(function: &str, input: &Value) -> HelperResult {
    match function {
        "well_known_root_suffix" => Ok(json!({
            "value": well_known_root_suffix(support::optional(input, "server_root_path").unwrap_or_default())
        })),
        "route_relative_path" => Ok(json!({
            "value": route_relative_request_path(
                support::optional(input, "original_path"),
                support::optional(input, "path").unwrap_or_default(),
                support::optional(input, "app_root_path"),
                support::optional(input, "root_path"),
            )
        })),
        _ => authenticate(function, input),
    }
}

fn authenticate(function: &str, input: &Value) -> HelperResult {
    match function {
        "metadata_url" => Ok(json!({"value": metadata(input)})),
        "www_authenticate" => Ok(json!({
            "value": passthrough_www_authenticate(
                &metadata(input),
                support::boolean(input, "invalid_token")?,
            )
        })),
        "byok_authenticate" => Ok(json!({
            "value": byok_www_authenticate(
                support::optional(input, "proxy_base_url").unwrap_or_default(),
                support::optional(input, "request_root_path").unwrap_or_default(),
            )
        })),
        other => Err(support::invalid_operation(other)),
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
