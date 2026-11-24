#![cfg(test)]

mod base_url;
mod document;
mod http_server;
mod input_schema;
mod naming;
mod parameters;
mod path_values;
mod redirects;
mod request_headers;
mod spec_loading;
mod support;
mod tool_call;
mod upstream_errors;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::openapi_tools::base_url::get_base_url;
use crate::openapi_tools::document::parse_openapi_document;
use crate::openapi_tools::input_schema::build_input_schema;
use crate::openapi_tools::parameters::{extract_parameters, resolve_operation_params};
use crate::openapi_tools::tool::{OpenApiTool, OpenApiToolConfig};

#[rstest]
fn replays_the_openapi_spec_corpus(#[files("fuzz/corpus/openapi_spec/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let Ok(document) = parse_openapi_document(&bytes) else {
        return;
    };
    drop(get_base_url(
        &document,
        Some("https://example.test/api/openapi.json"),
    ));
    for (path, path_item) in &document.paths {
        for (method, operation) in &path_item.operations {
            let resolved = resolve_operation_params(operation, path_item, &document.components);
            drop(extract_parameters(&resolved));
            drop(build_input_schema(&resolved));
            let _tool = OpenApiTool::new(OpenApiToolConfig {
                path: path.clone(),
                method: method.clone(),
                operation: resolved,
                base_url: "https://example.test".to_owned(),
                headers: BTreeMap::new(),
                server_label: None,
                relays_upstream_auth: false,
                auth_type: None,
                upstream_token_header: None,
            });
        }
    }
}
