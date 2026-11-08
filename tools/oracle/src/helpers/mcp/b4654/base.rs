use betterlite_mcp::oauth_resource::base_url::{
    RequestOrigin, get_request_base_url, normalize_proxy_base_url, strip_default_port,
};
use serde_json::{Value, json};

use super::support;
use crate::helpers::HelperResult;

pub(super) fn dispatch(function: &str, input: &Value) -> HelperResult {
    match function {
        "strip_default_port" => Ok(json!({
            "netloc": strip_default_port(
                support::string(input, "scheme")?,
                support::string(input, "netloc")?,
            )
        })),
        "proxy_base_url" => Ok(json!({
            "value": normalize_proxy_base_url(support::string(input, "raw")?)
        })),
        "request_base_url" => Ok(json!({
            "base_url": get_request_base_url(
                support::optional(input, "proxy_base_url").unwrap_or_default(),
                &RequestOrigin {
                    base_url: support::string(input, "base_url")?,
                    x_forwarded_proto: support::optional(input, "x_forwarded_proto"),
                    x_forwarded_host: support::optional(input, "x_forwarded_host"),
                    x_forwarded_port: support::optional(input, "x_forwarded_port"),
                    from_trusted_proxy: support::boolean(input, "from_trusted_proxy")?,
                },
            )?
        })),
        other => Err(support::invalid_operation(other)),
    }
}
