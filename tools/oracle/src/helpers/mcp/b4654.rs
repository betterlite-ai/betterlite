mod base;
mod challenges;
mod headers;
mod paths;
mod resource;
mod support;

use serde_json::Value;

use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

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
            "strip_default_port" | "proxy_base_url" | "request_base_url" => {
                base::dispatch(function, input)
            }
            "well_known_root_suffix"
            | "route_relative_path"
            | "metadata_url"
            | "www_authenticate"
            | "byok_authenticate" => challenges::dispatch(function, input),
            "canonicalize" | "redact_url" | "resolve_resource" | "token_request" => {
                resource::dispatch(function, input)
            }
            "safe_headers" | "litellm_key" | "mcp_auth_header" | "server_auth" | "oauth2"
            | "list_header" => headers::dispatch(function, input),
            "target_names" | "target_selection" => paths::dispatch(function, input),
            other => Err(support::invalid_operation(other)),
        }
    }
}
