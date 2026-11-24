#![no_main]

use std::collections::BTreeMap;

use betterlite_mcp::openapi_tools::base_url::get_base_url;
use betterlite_mcp::openapi_tools::document::parse_openapi_document;
use betterlite_mcp::openapi_tools::input_schema::build_input_schema;
use betterlite_mcp::openapi_tools::parameters::{extract_parameters, resolve_operation_params};
use betterlite_mcp::openapi_tools::tool::{OpenApiTool, OpenApiToolConfig};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(document) = parse_openapi_document(bytes) else {
        return;
    };
    let base_url = get_base_url(&document, Some("https://example.test/api/openapi.json"));
    for (path, path_item) in &document.paths {
        for (method, operation) in &path_item.operations {
            let resolved = resolve_operation_params(operation, path_item, &document.components);
            let params = extract_parameters(&resolved);
            let schema = build_input_schema(&resolved);
            let tool = OpenApiTool::new(OpenApiToolConfig {
                path: path.clone(),
                method: method.clone(),
                operation: resolved,
                base_url: base_url.clone(),
                headers: BTreeMap::new(),
                server_label: None,
                relays_upstream_auth: false,
                auth_type: None,
                upstream_token_header: None,
            });
            drop((params, schema, tool));
        }
    }
});
