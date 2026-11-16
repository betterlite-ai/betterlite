#![no_main]

use std::collections::BTreeSet;

use betterlite_config::sections::mcp_env_values::StdioPolicy;
use betterlite_mcp::connector_import::convert::convert_connector_entries;
use betterlite_mcp::connector_import::request::ConnectorImportRequest;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(request) = ConnectorImportRequest::from_slice(bytes) else {
        return;
    };
    let policy = StdioPolicy {
        enabled: true,
        allowed_commands: BTreeSet::from(["node".to_owned(), "npx".to_owned()]),
    };
    let _outcomes = convert_connector_entries(&request, &policy);
});
