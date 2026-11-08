use betterlite_mcp::request_headers::path_targets::{
    cold_start_server_names, resolve_target_server_names, target_names_from_path,
};
use serde_json::{Value, json};

use super::support;
use crate::helpers::HelperResult;

pub(super) fn dispatch(function: &str, input: &Value) -> HelperResult {
    match function {
        "target_names" => {
            let values: Vec<Vec<String>> = support::strings(input, "paths")
                .iter()
                .map(|path| target_names_from_path(path))
                .collect();
            Ok(json!({"values": values}))
        }
        "target_selection" => {
            let servers = header_servers(input);
            let path = support::string(input, "path")?;
            if support::optional(input, "mode") == Some("cold_start") {
                let cold = cold_start_server_names(path, servers.as_deref());
                return Ok(json!({"value": cold.map(|name| vec![name])}));
            }
            Ok(json!({
                "value": resolve_target_server_names(path, servers.as_deref())
            }))
        }
        other => Err(support::invalid_operation(other)),
    }
}

fn header_servers(input: &Value) -> Option<Vec<String>> {
    let names = support::optional_strings(input, "header_servers")?;
    let mut servers = Vec::with_capacity(names.len());
    for name in names {
        servers.push(name.to_owned());
    }
    Some(servers)
}
