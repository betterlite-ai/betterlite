use serde_json::json;

mod input;
mod operations;
mod tests;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::mcp::catalog_guard;
use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::StripPrefix { name, server } => {
                Ok(operations::run_prefix(name, server.as_ref()))
            }
            Input::SanitizeHeaders {
                headers,
                upstream_credential_names,
                identity_header_names,
                client_side_auth_header_name,
                custom_key_header_name,
            } => operations::sanitize_headers(
                headers,
                upstream_credential_names,
                identity_header_names,
                client_side_auth_header_name,
                custom_key_header_name.as_deref(),
            ),
            Input::Pin { tools, pinned } => operations::run_pin(tools, pinned),
            Input::Override {
                tools,
                overrides,
                prefix,
            } => operations::run_override(tools, overrides, prefix.as_deref()),
            Input::Scan {
                tools,
                outcomes,
                upstream_credential_names,
                headers,
                batch_report,
            } => operations::run_scan(
                tools,
                outcomes,
                upstream_credential_names,
                headers,
                *batch_report,
            ),
            Input::Error { failure } => Ok(json!({
                "reason": catalog_guard::block_reason(failure)
            })),
        }
    }
}
